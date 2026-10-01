#!/bin/bash
# Regenerate the Xcode diff themes from Apple's Default (Dark)/(Light) palettes:
#   scripts/gen-xcode-theme.sh dark > assets/diff-themes/xcode-dark.tmTheme
#   scripts/gen-xcode-theme.sh light > assets/diff-themes/xcode-light.tmTheme
set -e
v=$1
# The changed-words fills' opacity over their line's band, as hex: 33 = 20%, 40 = 25%, 59 = 35%, 66 = 40%, 80 = 50%,
# 8C = 55%, 99 = 60%, CC = 80%, FF = Xcode's full strength. Deleted words (orange), then added words (blue).
DEL_ALPHA=99
INS_ALPHA=CC
# The changed lines' bands: the same hues over the background, 26 = 15% orange, 40 = 25% blue.
DEL_LINE_ALPHA=26
INS_LINE_ALPHA=40
if [ "$v" = dark ]; then
  NAME="Xcode Default (Dark)"; BG=#292A30; FG=#DFDFE0; LINE=#2F3239; SEL=#646F83; GUT=#747478; CARET=#FFFFFF
  KW=#FF7AB2; STR=#FF8170; NUM=#D9C97C; CMT=#7F8C98; DOCKW=#A3B1BF; TDECL=#6BDFFF; ODECL=#4EB0CC
  PTYPE=#ACF2E4; PMEM=#78C2B3; STYPE=#DABAFF; SFUNC=#B281EB; PRE=#FFA14F; ATTR=#CC9768; URL=#6699FF; MARK=#92A1B1
  # Xcode's comparison view, sampled from a screenshot: a faint orange band under a deleted line
  # with its text on orange, a faint blue band under an added line with its changes on blue.
  DEL_LINE=#745238$DEL_LINE_ALPHA; DEL_TEXT=#745238$DEL_ALPHA; INS_LINE=#1C4872$INS_LINE_ALPHA; INS_TEXT=#1C4872$INS_ALPHA; DEL_ACCENT=#E8935A; INS_ACCENT=#4C9BFF
else
  NAME="Xcode Default (Light)"; BG=#FFFFFF; FG=#262626; LINE=#E8F2FF; SEL=#A4CDFF; GUT=#A6A6A6; CARET=#000000
  KW=#9B2393; STR=#C41A16; NUM=#1C00CF; CMT=#5D6C79; DOCKW=#4A5560; TDECL=#0B4F79; ODECL=#0F68A0
  PTYPE=#1C464A; PMEM=#326D74; STYPE=#3900A0; SFUNC=#6C36A9; PRE=#643820; ATTR=#815F03; URL=#0E0EFF; MARK=#4A5560
  # The same roles in light: no reference screenshot, so the dark hues lightened to match.
  DEL_LINE=#F6D7BC$DEL_LINE_ALPHA; DEL_TEXT=#F6D7BC$DEL_ALPHA; INS_LINE=#C4DCF7$INS_LINE_ALPHA; INS_TEXT=#C4DCF7$INS_ALPHA; DEL_ACCENT=#B5561A; INS_ACCENT=#1A6FD8
fi
rule() { # name scope fg [fontStyle]
  printf '\t\t<dict>\n\t\t\t<key>name</key>\n\t\t\t<string>%s</string>\n\t\t\t<key>scope</key>\n\t\t\t<string>%s</string>\n\t\t\t<key>settings</key>\n\t\t\t<dict>\n\t\t\t\t<key>foreground</key>\n\t\t\t\t<string>%s</string>\n' "$1" "$2" "$3"
  [ -n "$4" ] && printf '\t\t\t\t<key>fontStyle</key>\n\t\t\t\t<string>%s</string>\n' "$4"
  printf '\t\t\t</dict>\n\t\t</dict>\n'
}
cat <<HEAD
<?xml version="1.0" encoding="UTF-8"?>
<!--
$NAME
Hand-authored for diff-reckoner from the colors of Apple's built-in Xcode "$NAME"
source-editor theme (sRGB values, as Xcode 15 paints them), mapped onto TextMate scopes.
Xcode colors some roles from semantic analysis (project vs system symbols) that regex
grammars cannot see; those roles map to the nearest scope.
Licence: MIT (diff-reckoner). The color values are Apple's.
-->
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>name</key>
	<string>$NAME</string>
	<key>settings</key>
	<array>
		<dict>
			<key>settings</key>
			<dict>
				<key>background</key>
				<string>$BG</string>
				<key>foreground</key>
				<string>$FG</string>
				<key>caret</key>
				<string>$CARET</string>
				<key>lineHighlight</key>
				<string>$LINE</string>
				<key>selection</key>
				<string>$SEL</string>
				<key>gutterForeground</key>
				<string>$GUT</string>
				<key>diffDeleted</key>
				<string>$DEL_LINE</string>
				<key>diffDeletedText</key>
				<string>$DEL_TEXT</string>
				<key>diffInserted</key>
				<string>$INS_LINE</string>
				<key>diffInsertedText</key>
				<string>$INS_TEXT</string>
			</dict>
		</dict>
HEAD
rule "Comments" "comment, punctuation.definition.comment" $CMT
rule "Documentation markup keywords" "comment keyword, comment storage.type, comment.block.documentation keyword, comment entity.name.tag" $DOCKW bold
rule "Marks" "comment.line.mark, meta.toc-list" $MARK bold
rule "Keywords" "keyword, storage.type, storage.modifier, constant.language, variable.language, keyword.other.statement" $KW bold
rule "Operators and punctuation are plain text" "keyword.operator, punctuation, meta.brace, storage.type.function.arrow" $FG " "
rule "Strings" "string, punctuation.definition.string" $STR
rule "String interpolation is code" "meta.interpolation, string meta.embedded, string source, meta.literal.string.swift meta.expression.swift, string punctuation.section.interpolation, support.punctuation.expression" $FG " "
rule "Characters and escapes" "constant.character, constant.character.escape, constant.other.placeholder" $NUM
rule "Numbers" "constant.numeric" $NUM
rule "Type declarations" "entity.name.type.class, entity.name.type.struct, entity.name.type.enum, entity.name.type.protocol, entity.name.type.interface, entity.name.class, entity.name.struct, entity.name.enum, entity.name.union, entity.name.trait, entity.name.interface, entity.name.protocol, entity.name.impl, entity.name.type.alias" $TDECL
rule "Other declarations" "entity.name.function, variable.parameter, entity.name.variable, entity.name.constant, entity.name.enum-member, variable.other.enummember, meta.definition.variable variable.other, variable.other.declaration" $ODECL
rule "Project types" "entity.name.type, entity.other.inherited-class, entity.name.type.inherited, meta.inheritance entity.other, storage.type.class, entity.name.type.class.usage" $PTYPE
rule "Project members" "variable.other.member, variable.other.property, variable.other.object.property, meta.property.object, support.variable.property, variable.other.field" $PMEM
rule "System types" "support.type, support.class, storage.type.primitive, storage.type.built-in, entity.name.type.primitive, storage.type.numeric, storage.type.boolean, storage.type.string" $STYPE
rule "System functions" "support.function, variable.function, meta.function-call entity.name.function, entity.name.function.call" $SFUNC
rule "Modules are plain text" "support.type.module, entity.name.namespace, entity.name.module, entity.name.package, meta.import support.type, meta.path entity.name.namespace" $FG " "
rule "Preprocessor and macros" "meta.preprocessor, keyword.control.directive, keyword.control.import.c, keyword.control.import.include, support.macro, entity.name.function.macro, entity.name.function.preprocessor, entity.name.macro" $PRE
rule "Attributes" "storage.modifier.attribute, meta.attribute, meta.annotation, entity.other.attribute-name.swift, punctuation.definition.attribute, meta.decorator, punctuation.definition.annotation" $ATTR " "
rule "URLs" "markup.underline.link, constant.other.reference.link, string.other.link" $URL "underline"
rule "Markup headings" "markup.heading, entity.name.section" $KW bold
rule "Markup emphasis" "markup.bold" $FG bold
rule "Markup italic" "markup.italic" $FG italic
rule "Markup code" "markup.raw, markup.inline.raw" $STR
rule "Tags" "entity.name.tag" $KW bold
rule "Tag attributes" "entity.other.attribute-name" $ATTR " "
rule "Diff deletions" "markup.deleted" $DEL_ACCENT
rule "Diff insertions" "markup.inserted" $INS_ACCENT
cat <<'TAIL'
	</array>
</dict>
</plist>
TAIL
