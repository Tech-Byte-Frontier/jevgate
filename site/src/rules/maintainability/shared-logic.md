# Shared logic

{{#include ../../reference/_rules.md:maintainability-shared-logic}}

## When a finding is right

A finding says two or more places may repeat one piece of logic, so a change to it would have to be made in each place. Code finds the candidates without asking: renamed copies of statement windows, and runs of at least 12 tokens, three of them words, repeated in two to twelve places, with local names made alike and member names and literals kept. Runs read the statements the windows read, less a traversal's frame and Go's error checks, and functions of two statements or fewer are left out as thin wrappers. Jev is shown up to six sites of each repeat, with their paths and functions, and asked whether they express the same rule, lookup or sequence of steps. It is right when the copies would change together: the same eligibility check in two files, or two functions that open, configure, watch and close a connection the same way. It is wrong when the resemblance is incidental: calls every user of a library writes the same way, complementary operations such as encode and decode, or test cases that set up their own data.

Copies are compared within a package and across packages linked by a local dependency, copies inside example code are notes, which are not reported, and copies in code marked deprecated are not compared. A repeat whose every site lies inside tests a test-redundancy finding names is reported by that finding alone.

## How it is measured

A look-here finding has not been labeled yet, so it says `Not yet measured.` and never fails the check by default. Each one a coding agent or person dismisses with a reason (`jevgate baseline mark wrong|intended|later PATH:LINE`) is counted by `jevgate baseline stats`, which is how its share of noise shows in daily use.
