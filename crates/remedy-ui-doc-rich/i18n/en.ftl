# The rich document widgets' own words. Placeholders ({ $name }) must stay; their order may
# change. Keyboard hints name keys as the keyboard prints them in the language.

## The rich text editor.
# The editor's name for screen readers when the app gives it none (a chat's composer).
editor-label = Message
# The formatting toolbar's buttons.
format-bold = Bold
format-italic = Italic
format-strikethrough = Strikethrough
# Inline code (monospace text).
format-code = Code
format-heading = Heading
format-bulleted-list = Bulleted list
format-numbered-list = Numbered list
# A list of items with check boxes.
format-task-list = Task list
format-quote = Quote
format-code-block = Code block
format-link = Link
# The toolbar, as a group, for screen readers.
formatting-toolbar = Formatting

## The link box (Ctrl+K).
link-address = Link address
# Sets the address typed on the selected text.
link-apply = Apply
link-remove = Remove link
# The box, as a dialog, for screen readers.
link-dialog = Link

## Suggestions of people to mention after "@".
mention-count = { $count ->
    [one] { $count } person to mention
   *[other] { $count } people to mention
}
mention-keys = Up and Down choose, Enter or Tab mentions, Escape closes

## The find bar.
# The search field's name and the grey hint in it.
find-label = Find
find-placeholder = Find
# The whole bar, as a group, for screen readers.
find-group = Find
find-no-matches = No matches
# Which match is selected: "3 of 12".
find-position = { $current } of { $count }
find-previous = Previous match
find-next = Next match
find-close = Close find

## Markdown shown as text, for screen readers.
# A block quote.
markdown-quote = Quote
# A code block; { $language } is the language it is in, as written ("rust").
markdown-code = Code
markdown-code-language = Code, { $language }
