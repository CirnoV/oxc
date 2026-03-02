# Exit code
1

# stdout
```
  x message-id-plugin(no-var): Unexpected var, use let or const instead.
   ,-[files/index.js:1:1]
 1 | var reportUsingNode = 1;
   : ^^^^^^^^^^^^^^^^^^^^^^^^
 2 | var reportUsingRange = 1;
   `----

  x message-id-plugin(no-var): Unexpected var, use let or const instead.
   ,-[files/index.js:2:1]
 1 | var reportUsingNode = 1;
 2 | var reportUsingRange = 1;
   : ^^^^^^^^^^^^^^^^^^^^^^^^^
   `----

Found 2 errors in 1 file.

Errors  Files
     2  files/index.js:1

Finished in Xms on 1 file with 1 rules using X threads.
```

# stderr
```
```
