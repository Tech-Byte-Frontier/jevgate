; Bash (tree-sitter-bash ships no tags query): functions, and commands, the
; calls of a shell.
(function_definition name: (word) @name) @definition.function

(command name: (command_name (word) @name)) @reference.call
