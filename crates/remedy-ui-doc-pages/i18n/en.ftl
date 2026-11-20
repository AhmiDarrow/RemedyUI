# The page view's own words. Placeholders ({ $name }) must stay; their order may change.
# { $kind } selects an annotation's kind: highlight, underline, strikeout, squiggly
# (a wavy underline), drawing (freehand ink), note (a sticky note), text-box, rectangle,
# ellipse, line, arrow; "annotation" when it is not known.

## Pages.
# A page, for screen readers and the thumbnail rail.
page-number = Page { $page }
# The thumbnail rail, as a list.
pages-list = Pages
# What a screen reader hears for the whole view: the current page, the page count and the
# zoom in percent.
page-view-value = Page { $page } of { $count }, { $zoom }%

## Annotations, as screen readers hear them.
# Its kind, in a word, at the start of a description.
annotation-kind = { $kind ->
    [highlight] Highlight
    [underline] Underline
    [strikeout] Strikeout
    [squiggly] Squiggly underline
    [drawing] Drawing
    [note] Note
    [text-box] Text box
    [rectangle] Rectangle
    [ellipse] Ellipse
    [line] Line
    [arrow] Arrow
   *[annotation] Annotation
}
# { $what } is the annotation so far ("Highlight"); { $text } the text it marks or holds.
annotation-quoting = { $what }: “{ $text }”
annotation-with-comment = { $what }, comment: { $comment }
annotation-by = { $what }, by { $author }
# The selected annotation; { $description } is its description.
annotation-selected = Selected: { $description }
# Heads the list of the page's annotations.
annotation-count = { $count ->
    [one] { $count } annotation:
   *[other] { $count } annotations:
}

## Undo history names (shown as "Undo Add highlight" in the Edit menu).
undo-add = { $kind ->
    [highlight] Add highlight
    [underline] Add underline
    [strikeout] Add strikeout
    [squiggly] Add squiggly underline
    [drawing] Add drawing
    [note] Add note
    [text-box] Add text box
    [rectangle] Add rectangle
    [ellipse] Add ellipse
    [line] Add line
    [arrow] Add arrow
   *[annotation] Add annotation
}
undo-change = { $kind ->
    [highlight] Change highlight
    [underline] Change underline
    [strikeout] Change strikeout
    [squiggly] Change squiggly underline
    [drawing] Change drawing
    [note] Change note
    [text-box] Change text box
    [rectangle] Change rectangle
    [ellipse] Change ellipse
    [line] Change line
    [arrow] Change arrow
   *[annotation] Change annotation
}
undo-delete = { $kind ->
    [highlight] Delete highlight
    [underline] Delete underline
    [strikeout] Delete strikeout
    [squiggly] Delete squiggly underline
    [drawing] Delete drawing
    [note] Delete note
    [text-box] Delete text box
    [rectangle] Delete rectangle
    [ellipse] Delete ellipse
    [line] Delete line
    [arrow] Delete arrow
   *[annotation] Delete annotation
}
undo-move = { $kind ->
    [highlight] Move highlight
    [underline] Move underline
    [strikeout] Move strikeout
    [squiggly] Move squiggly underline
    [drawing] Move drawing
    [note] Move note
    [text-box] Move text box
    [rectangle] Move rectangle
    [ellipse] Move ellipse
    [line] Move line
    [arrow] Move arrow
   *[annotation] Move annotation
}
