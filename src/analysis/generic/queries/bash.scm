; Bash (tree-sitter-bash ships no tags query): functions, commands, the
; calls of a shell, and the files a script reads in with `source` or `.`,
; the only way two scripts share code.
(function_definition name: (word) @name) @definition.function

(command name: (command_name (word) @name)) @reference.call

(command
  name: (command_name (word) @command)
  .
  argument: (_) @include
  (#any-of? @command "source" "."))
