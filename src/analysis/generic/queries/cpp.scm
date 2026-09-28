; C++: functions and methods, their declarator reached through the pointer
; or reference they return; a qualified name (`Cart::add`, `ns::Cart::add`)
; names the type it is written in, and an operator (`operator==`) itself;
; templates with their `template` line; classes, structs, unions, enums and
; typedefs; and calls.
(function_definition
  declarator: [
    (function_declarator declarator: [
      (identifier) (field_identifier) (destructor_name) (operator_name)
      (qualified_identifier) (template_function)] @name)
    (pointer_declarator declarator: (function_declarator declarator: [
      (identifier) (field_identifier) (operator_name) (qualified_identifier)] @name))
    (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator
      declarator: [(identifier) (field_identifier) (qualified_identifier)] @name)))
    (reference_declarator (function_declarator declarator: [
      (identifier) (field_identifier) (operator_name) (qualified_identifier)] @name))
  ]) @definition.function
(template_declaration
  (function_definition
    declarator: [
      (function_declarator declarator: [
        (identifier) (field_identifier) (operator_name) (qualified_identifier)] @name)
      (pointer_declarator declarator: (function_declarator declarator: [
        (identifier) (field_identifier) (qualified_identifier)] @name))
      (reference_declarator (function_declarator declarator: [
        (identifier) (field_identifier) (operator_name) (qualified_identifier)] @name))
    ]
    body: (_) @body)) @definition.function

(class_specifier name: (type_identifier) @name body: (_)) @definition.class
(struct_specifier name: (type_identifier) @name body: (_)) @definition.class
(union_specifier name: (type_identifier) @name body: (_)) @definition.class
(enum_specifier name: (type_identifier) @name body: (_)) @definition.type
(type_definition declarator: (type_identifier) @name) @definition.type

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (field_expression field: (field_identifier) @name)) @reference.call
(call_expression function: (qualified_identifier name: (identifier) @name)) @reference.call
(call_expression function: (template_function name: (identifier) @name)) @reference.call
