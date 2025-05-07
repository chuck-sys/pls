# Features

A long list of things I want to have.

## Hover

- [ ] Show hover text (phpdocs)

## Go to

- [ ] Go to definition
- [ ] Go to declaration
- [ ] Go to type

## Completion

- [ ] Auto-complete with symbols, functions, methods, classes, etc.
- [ ] Smart auto-complete that prioritizes items that fit (e.g. if a function parameter requires an `int`
  then it would put symbols that match the type `int` at the top)

## Code actions

- [x] Convert most forms of `<?php echo ... ?>` into `<?= ... ?>`
- [ ] Convert function calls into function calls with named parameters, while removing redundant default
  arguments
- [ ] Convert an applicable for loop over an array into an equivalent foreach loop
- [ ] Refactor selected code into a function

## Diagnostics

- [ ] Undefined variables
    - [x] Variables that aren't assigned
    - [ ] Variables created via a pass-by-reference function call
- [ ] Warn of variables created in a loop that could be moved out of it

## Feature flags

- [ ] Make sure that certain categories of diagnostics can be disabled via configurations
