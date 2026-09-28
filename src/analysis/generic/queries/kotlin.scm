; Kotlin (tree-sitter-kotlin-ng ships no tags query): classes, interfaces
; and enums, objects, functions with their body (a block, or an expression
; after `=`), `init` blocks, secondary constructors, properties with a
; getter or setter, and calls.
(class_declaration name: (identifier) @name) @definition.class
(object_declaration name: (identifier) @name) @definition.object
(function_declaration
  name: (identifier) @name
  (function_body)? @body) @definition.function
(anonymous_initializer "init" @name (block) @body) @definition.function
(secondary_constructor "constructor" @name (block) @body) @definition.function
(property_declaration
  (variable_declaration (identifier) @name)
  [(getter (function_body) @body) (setter (function_body) @body)]) @definition.function

(call_expression . (identifier) @name) @reference.call
(call_expression . (navigation_expression (identifier) @name .)) @reference.call
