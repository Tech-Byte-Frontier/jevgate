; Swift: classes, structs, enums, actors and extensions (named by the type
; they extend), protocols, functions, initializers and deinitializers,
; computed properties (a SwiftUI view's `body`) and subscripts, and calls.
(class_declaration name: (type_identifier) @name) @definition.class
(class_declaration name: (user_type (type_identifier) @name .)) @definition.class
(protocol_declaration name: (type_identifier) @name) @definition.interface
(function_declaration name: (simple_identifier) @name) @definition.function
(protocol_function_declaration name: (simple_identifier) @name) @definition.function
(init_declaration "init" @name) @definition.function
(deinit_declaration "deinit" @name) @definition.function
(property_declaration
  name: (pattern (simple_identifier) @name)
  computed_value: (computed_property) @body) @definition.function
(subscript_declaration "subscript" @name (computed_property) @body) @definition.function

(call_expression . (simple_identifier) @name) @reference.call
(call_expression
  . (navigation_expression suffix: (navigation_suffix suffix: (simple_identifier) @name))) @reference.call
