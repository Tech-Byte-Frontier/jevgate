; Elixir: modules, protocols and implementations, functions and macros with
; the `do` block of their body (one-line `do:` clauses have none), and
; calls, local, remote and piped.
(call
  target: (identifier) @keyword
  (arguments . (alias) @name)
  (#any-of? @keyword "defmodule" "defprotocol" "defimpl")) @definition.module
(call
  target: (identifier) @keyword
  (arguments
    .
    [(identifier) @name
     (call target: (identifier) @name)
     (binary_operator left: (call target: (identifier) @name) operator: "when")])
  (do_block)? @body
  (#any-of? @keyword "def" "defp" "defmacro" "defmacrop" "defguard" "defguardp" "defn" "defnp")) @definition.function

(call target: (identifier) @name) @reference.call
(call target: (dot right: (identifier) @name)) @reference.call
(binary_operator operator: "|>" right: (identifier) @name) @reference.call
