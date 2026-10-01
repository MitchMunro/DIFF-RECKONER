; diff-reckoner's additions to the Helix query beside it (MIT). Loaded after it, so a
; pattern here wins over Helix's for the same node.

; Declared names, in a color different from their uses.
(class_declaration (type_identifier) @type.declaration)
(object_declaration (type_identifier) @type.declaration)
(function_declaration (simple_identifier) @function.declaration)
(property_declaration (variable_declaration (simple_identifier) @variable.declaration))
(multi_variable_declaration (variable_declaration (simple_identifier) @variable.declaration))
(enum_entry (simple_identifier) @variable.declaration)

; Annotations (`@Composable`) in one color, name and all.
(annotation "@" @attribute)
(annotation (user_type (type_identifier) @attribute))
(annotation (constructor_invocation (user_type (type_identifier) @attribute)))
