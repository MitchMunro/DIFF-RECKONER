; diff-reckoner's additions to the Helix query beside it (MIT). Loaded after it, so a
; pattern here wins over Helix's for the same node.

; Declared names, in a color different from their uses. Helix already captures parameters.
(class_declaration name: (type_identifier) @type.declaration)
(protocol_declaration name: (type_identifier) @type.declaration)
(typealias_declaration name: (type_identifier) @type.declaration)
(function_declaration name: (simple_identifier) @function.declaration)
(protocol_function_declaration name: (simple_identifier) @function.declaration)
(property_declaration name: (pattern bound_identifier: (simple_identifier) @variable.declaration))
(protocol_property_declaration name: (pattern bound_identifier: (simple_identifier) @variable.declaration))
(enum_entry name: (simple_identifier) @variable.declaration)

; `init` and `deinit` are keywords to Xcode, not names.
(init_declaration "init" @keyword)
(deinit_declaration "deinit" @keyword)

; Attributes (`@MainActor`) in one color, name and all.
(attribute "@" @attribute)
(attribute (user_type (type_identifier) @attribute))

; The standard library, Foundation and SwiftUI types Xcode colors as system types.
((type_identifier) @type.builtin
  (#any-of? @type.builtin
    "Any" "AnyObject" "AnyHashable" "Array" "Binding" "Bool" "Bundle" "Calendar"
    "CaseIterable" "CGFloat" "CGPoint" "CGRect" "CGSize" "Character" "ClosedRange" "Codable"
    "Collection" "Color" "Comparable" "Data" "Date" "DateFormatter" "Decodable" "Dictionary"
    "DispatchQueue" "Double" "Encodable" "Environment" "EnvironmentObject" "Equatable"
    "Error" "FileManager" "Float" "Font" "ForEach" "HStack" "Hashable" "Identifiable" "Image"
    "Int" "JSONDecoder" "JSONEncoder" "List" "LocalizedError" "Locale" "MainActor"
    "NavigationStack" "NavigationView" "Never" "NotificationCenter" "ObservableObject"
    "ObservedObject" "Optional" "Published" "Range" "Result" "Sendable" "Sequence" "Set"
    "Spacer" "State" "StateObject" "String" "Substring" "Task" "Text" "TimeInterval"
    "TimeZone" "URL" "UUID" "UserDefaults" "VStack" "View" "Void" "ZStack"))
