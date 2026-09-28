; Dart: classes, mixins, extensions and enums, top-level functions,
; getters and setters, methods (constructors, getters and setters among
; them), and calls.
(class_declaration name: (identifier) @name) @definition.class
(mixin_declaration name: (identifier) @name) @definition.class
(extension_declaration name: (identifier) @name) @definition.class
(enum_declaration name: (identifier) @name) @definition.class
(function_declaration signature: (function_signature name: (identifier) @name)) @definition.function
(getter_declaration signature: (getter_signature name: (identifier) @name)) @definition.function
(setter_declaration signature: (setter_signature name: (identifier) @name)) @definition.function
(method_declaration
  signature: (method_signature
    [(function_signature name: (identifier) @name)
     (getter_signature name: (identifier) @name)
     (setter_signature name: (identifier) @name)
     (constructor_signature name: (identifier) @name)
     (factory_constructor_signature . (identifier) @name)])) @definition.method

(call_expression function: (identifier) @name) @reference.call
(call_expression function: (member_expression property: (identifier) @name)) @reference.call
