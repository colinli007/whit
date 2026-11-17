; Added by Whit, adapted from tree-sitter-javascript (MIT, Copyright (c) 2014
; Max Brunsfeld); see THIRD-PARTY-NOTICES.md in the Whit repository.
(jsx_opening_element (identifier) @tag (#match? @tag "^[a-z][^.]*$"))
(jsx_closing_element (identifier) @tag (#match? @tag "^[a-z][^.]*$"))
(jsx_self_closing_element (identifier) @tag (#match? @tag "^[a-z][^.]*$"))

(jsx_opening_element (identifier) @tag.component (#match? @tag.component "^[A-Z]"))
(jsx_closing_element (identifier) @tag.component (#match? @tag.component "^[A-Z]"))
(jsx_self_closing_element (identifier) @tag.component (#match? @tag.component "^[A-Z]"))

(jsx_attribute (property_identifier) @attribute)
(jsx_opening_element (["<" ">"]) @tag.delimiter)
(jsx_closing_element (["</" ">"]) @tag.delimiter)
(jsx_self_closing_element (["<" "/>"]) @tag.delimiter)
(jsx_expression ["{" "}"] @punctuation.section.embedded)
