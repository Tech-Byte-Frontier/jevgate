; Kotlin (tree-sitter-kotlin-ng ships no tags query): classes, interfaces
; and enums, objects, functions with their body (a block, or an expression
; after `=`), and calls.
(class_declaration name: (identifier) @name) @definition.class
(object_declaration name: (identifier) @name) @definition.object
(function_declaration
  name: (identifier) @name
  (function_body)? @body) @definition.function

(call_expression . (identifier) @name) @reference.call
(call_expression . (navigation_expression (identifier) @name .)) @reference.call
