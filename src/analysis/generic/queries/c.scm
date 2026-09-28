; C: functions (returning values, pointers or pointers to pointers), the
; structs, unions and enums defined with a body, typedefs, and calls.
(function_definition
  declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(function_definition
  declarator: (pointer_declarator
    declarator: (pointer_declarator
      declarator: (function_declarator declarator: (identifier) @name)))) @definition.function

(struct_specifier name: (type_identifier) @name body: (_)) @definition.class
(union_specifier name: (type_identifier) @name body: (_)) @definition.class
(enum_specifier name: (type_identifier) @name body: (_)) @definition.type
(type_definition declarator: (type_identifier) @name) @definition.type

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression field: (field_identifier) @name)) @reference.call
