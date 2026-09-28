; Scala: classes, case classes, objects, traits and enums, functions
; (abstract ones in traits too, and operators such as `def /`), and calls.
(class_definition name: (identifier) @name) @definition.class
(object_definition name: (identifier) @name) @definition.object
(trait_definition name: (identifier) @name) @definition.interface
(enum_definition name: (identifier) @name) @definition.class
(function_definition name: (_) @name) @definition.function
(function_declaration name: (_) @name) @definition.function

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression field: (identifier) @name)) @reference.call
