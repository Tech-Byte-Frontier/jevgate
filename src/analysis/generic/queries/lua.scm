; Lua: functions, those of a table (`function M.add`, `function M:send`,
; `M.run = function`) owned by it, functions assigned to a name or held in
; a table field, and calls.
(function_declaration name: (identifier) @name) @definition.function
(function_declaration
  name: (dot_index_expression table: (_) @scope field: (identifier) @name)) @definition.function
(function_declaration
  name: (method_index_expression table: (_) @scope method: (identifier) @name)) @definition.method
(assignment_statement
  (variable_list . name: (identifier) @name)
  (expression_list . value: (function_definition body: (block)? @body))) @definition.function
(assignment_statement
  (variable_list . name: (dot_index_expression table: (_) @scope field: (identifier) @name))
  (expression_list . value: (function_definition body: (block)? @body))) @definition.function
(field
  name: (identifier) @name
  value: (function_definition body: (block)? @body)) @definition.function

(function_call
  name: [(identifier) @name
         (dot_index_expression field: (identifier) @name)
         (method_index_expression method: (identifier) @name)]) @reference.call
