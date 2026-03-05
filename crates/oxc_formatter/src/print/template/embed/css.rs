use oxc_allocator::StringBuilder;
use oxc_ast::ast::*;

use crate::{
    ast_nodes::AstNode,
    external_formatter::EmbeddedDocResult,
    format_args,
    formatter::{FormatElement, Formatter, format_element::TextWidth, prelude::*},
    write,
};

/// Format a CSS-in-JS template literal via the Doc→IR path with placeholder replacement.
///
/// Joins quasis with `@prettier-placeholder-N-id` markers, formats as SCSS,
/// then replaces placeholder occurrences in the resulting IR with `${expr}` Docs.
///
/// Works for both tagged templates (`css`...``, `styled.div`...``) and
/// styled-jsx (`<style jsx>{`...`}</style>`).
pub(super) fn format_css_doc<'a>(
    quasi: &AstNode<'a, TemplateLiteral<'a>>,
    f: &mut Formatter<'_, 'a>,
) -> bool {
    let quasis = &quasi.quasis;
    let num_quasis = quasis.len();
    let num_expressions = num_quasis - 1;

    // Build joined text: quasis[0].raw + "@prettier-placeholder-0-id" + quasis[1].raw + ...
    let allocator = f.context().allocator();
    let joined = if num_expressions == 0 {
        quasis[0].value.raw.as_str()
    } else {
        let mut sb = StringBuilder::new_in(allocator);
        for (idx, quasi_elem) in quasis.iter().enumerate() {
            if idx > 0 {
                sb.push_str("@prettier-placeholder-");
                push_usize(&mut sb, idx - 1);
                sb.push_str("-id");
            }
            sb.push_str(quasi_elem.value.raw.as_str());
        }
        sb.into_str()
    };

    // Empty template with no expressions → ``
    if joined.trim().is_empty() && num_expressions == 0 {
        write!(f, ["``"]);
        return true;
    }

    // Format via the Doc→IR path (CSS-specific: returns IR + placeholder count)
    let allocator = f.allocator();
    let group_id_builder = f.group_id_builder();
    let Some(Ok(EmbeddedDocResult::DocWithPlaceholders(ir, placeholder_count))) = f
        .context()
        .external_callbacks()
        .format_embedded_doc(allocator, group_id_builder, "tagged-css", &[joined])
    else {
        return false;
    };

    // Collect expressions via AstNode-aware iterator
    let expressions: Vec<_> = quasi.expressions().iter().collect();

    // No expressions → write IR as-is
    if expressions.is_empty() {
        let format_content = format_once(|f: &mut Formatter<'_, 'a>| {
            f.write_elements(ir);
        });
        write!(f, ["`", block_indent(&format_content), "`"]);
        return true;
    }

    // Verify all placeholders survived SCSS formatting.
    // Some edge cases (e.g. `/* prettier-ignore */` before a placeholder without semicolon)
    // cause SCSS to drop placeholders. In that case, fall back to regular template formatting
    // (same behavior as Prettier's `replacePlaceholders` returning null).
    if placeholder_count != expressions.len() {
        return false;
    }

    // Walk IR, replace placeholder Text nodes with expressions.
    // Consecutive Text nodes have already been merged by `from_prettier_doc::postprocess`,
    // so each placeholder appears as a complete `@prettier-placeholder-N-id` within a single Text.
    let indent_width = f.options().indent_width;
    let format_content = format_once(move |f: &mut Formatter<'_, 'a>| {
        for element in ir {
            match &element {
                FormatElement::Text { text, .. } if text.contains("@prettier-placeholder") => {
                    let parts = split_on_placeholders(text);
                    for (i, part) in parts.iter().enumerate() {
                        if i % 2 == 0 {
                            if !part.is_empty() {
                                write_text_with_line_breaks(f, part, allocator, indent_width);
                            }
                        } else if let Some(idx) = parse_usize(part)
                            && let Some(expr) = expressions.get(idx)
                        {
                            // Format ${expr} preserving soft line breaks so the printer
                            // can decide line breaks based on printWidth.
                            // (Regular template expressions use `RemoveSoftLinesBuffer`
                            // which forces single-line layout.)
                            write!(
                                f,
                                [group(&format_args!("${", expr, line_suffix_boundary(), "}"))]
                            );
                        }
                    }
                }
                _ => {
                    f.write_element(element);
                }
            }
        }
    });

    write!(f, ["`", block_indent(&format_content), "`"]);
    true
}

/// Emit text with newlines converted to literal line breaks (replaceEndOfLine equivalent).
///
/// Uses `Text("\n") + ExpandParent` (the literalline pattern) instead of `hard_line_break()`
/// to avoid adding indentation. The SCSS formatter has already computed proper indentation
/// in the text content, so we must not add extra indent from the surrounding `block_indent`.
fn write_text_with_line_breaks<'a>(
    f: &mut Formatter<'_, 'a>,
    text: &str,
    allocator: &'a oxc_allocator::Allocator,
    indent_width: crate::IndentWidth,
) {
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            // Emit literalline: Text("\n") + ExpandParent
            let newline = allocator.alloc_str("\n");
            f.write_element(FormatElement::Text { text: newline, width: TextWidth::multiline(0) });
            f.write_element(FormatElement::ExpandParent);
        }
        first = false;
        if !line.is_empty() {
            let arena_text = allocator.alloc_str(line);
            let width = TextWidth::from_text(arena_text, indent_width);
            f.write_element(FormatElement::Text { text: arena_text, width });
        }
    }
}

/// Split text on `@prettier-placeholder-N-id` patterns.
///
/// Returns alternating parts: `[literal, index_str, literal, index_str, ...]`
/// Similar to JavaScript `String.split(/(@prettier-placeholder-(\d+)-id)/)` but
/// only captures the digit group (index).
fn split_on_placeholders(text: &str) -> Vec<&str> {
    const PREFIX: &str = "@prettier-placeholder-";
    const SUFFIX: &str = "-id";

    let mut result = Vec::new();
    let mut remaining = text;

    loop {
        let Some(start) = remaining.find(PREFIX) else {
            result.push(remaining);
            break;
        };

        // Push the literal before the placeholder
        result.push(&remaining[..start]);

        // Skip past the prefix
        let after_prefix = &remaining[start + PREFIX.len()..];

        // Find the digits
        let digit_end =
            after_prefix.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(after_prefix.len());

        if digit_end == 0 {
            // No digits found after prefix — not a valid placeholder, treat as literal
            if let Some(last) = result.last_mut() {
                let end = start + PREFIX.len();
                *last = &remaining[..end];
            }
            remaining = &remaining[start + PREFIX.len()..];
            continue;
        }

        let digits = &after_prefix[..digit_end];
        let after_digits = &after_prefix[digit_end..];

        // Check for the `-id` suffix
        if let Some(after_suffix) = after_digits.strip_prefix(SUFFIX) {
            // Valid placeholder - push the digit index
            result.push(digits);
            remaining = after_suffix;
        } else {
            // Not a valid placeholder, include in the literal
            let end = start + PREFIX.len() + digit_end;
            if let Some(last) = result.last_mut() {
                *last = &remaining[..end];
            }
            remaining = &remaining[end..];
        }
    }

    result
}

/// Push a usize as decimal digits to a StringBuilder.
fn push_usize(sb: &mut StringBuilder<'_>, n: usize) {
    let _ = std::fmt::Write::write_fmt(sb, std::format_args!("{n}"));
}

/// Parse a decimal string to usize.
fn parse_usize(s: &str) -> Option<usize> {
    s.parse::<usize>().ok()
}
