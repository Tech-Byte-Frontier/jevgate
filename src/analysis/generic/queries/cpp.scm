; C++: C's definitions, methods written in their class, members defined
; outside it (`Cart::add`, owned by the scope they name), destructors,
; templates with their `template` line, classes, and calls.
(function_definition
  declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition
  declarator: (function_declarator declarator: (field_identifier) @name)) @definition.method
(function_definition
  declarator: (function_declarator declarator: (destructor_name) @name)) @definition.method
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier scope: (_) @scope name: [(identifier) (destructor_name)] @name))) @definition.method
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(function_definition
  declarator: (reference_declarator
    (function_declarator declarator: (identifier) @name))) @definition.function
(template_declaration
  (function_definition
    declarator: (function_declarator declarator: [(identifier) (field_identifier)] @name)
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
