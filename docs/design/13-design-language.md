# 13. Design language: good by default

People should feel at home in any Remedy app without reading anything. That comes from consistent behavior and from the platform conventions people already know. RemedyUI builds both into its widgets, so an app gets them without extra work. This document is the short version of the Remedy interface guidelines. Each rule names where the engine enforces or provides it.

## Feel

| Rule | Where it lives |
|---|---|
| Every action answers within 100 ms: a press state, a cursor, a spinner after 400 ms of waiting | Widget states; `suspense` and `progress` with built-in delays |
| Nothing jumps: content that loads late takes its space first (placeholders and estimated heights), and lists anchor | Layout anchoring; skeleton placeholders in `suspense` |
| Motion explains where things come from and go, and is short (100–300 ms) | Theme motion tokens; enter/exit and shared-element transitions |
| Destructive actions can be undone instead of asking "are you sure?" | Undo service; `toast` with an Undo action |
| Confirmation is kept for actions that cannot be undone, and says exactly what will happen | `dialog` with a `destructive` command; the button names the action ("Delete 3 files"), never "OK" |
| Errors say what happened and what to do, next to where it happened | Field-level `invalid` state and message; form validation |
| State is kept: window size and position, scroll position, drafts, selections, split sizes | `persist` signals, used by the built-in containers |

## Familiar

| Rule | Where it lives |
|---|---|
| Platform shortcuts work everywhere (copy, paste, undo, find, select all, close, zoom) | Bridge conventions + the command table |
| Text can be selected and copied wherever a user would expect to copy it | `selectable()` defaults on in documents, messages and read-only fields |
| Right click (or long press) on anything that has actions shows them | `context_menu` built from the element's commands |
| Every command with a shortcut shows it in its tooltip and menu entry | Command table |
| Dialogs use the platform's button order and close on Escape, or back on Android | Bridge conventions; `modal` |
| Scrolling feels like the platform's | Scroll physics per bridge |
| Drag and drop works where it looks like it should: files onto drop zones, items in reorderable lists | Built-in reorder; `file_drop_zone` |

## Clear

| Rule | Where it lives |
|---|---|
| One primary action per screen or dialog, visually first | `Variant::Primary`; a debug lint warns about two primaries in one scope |
| Labels are sentences, short, and say what happens ("Send invite", not "Submit") | Docs and review; default strings in widgets follow it |
| Icons have text labels or tooltips | `icon_button` requires a label (used for its tooltip and accessible name) |
| Status has text, not only color (a lamp has a label; an error has a message) | `lamp` and `badge` take labels; color-blind safe palettes in themes |
| Touch targets are at least 44–48 dp; mouse targets 24 dp | Hit-area extension ([11](11-accessibility-and-automation.md)) |
| Contrast meets WCAG AA at least | Theme derived tokens and contrast checks |

## Calm

| Rule | Where it lives |
|---|---|
| Nothing moves or blinks unless it means something | Decorative loops pause when idle; Reduced Motion |
| Notifications only for what the user asked to know about | `services-desktop` notifications are opt-in per category, and the user can turn categories off |
| Empty states say what goes here and how to start | `empty_state` widget |
| Defaults are sensible, and settings are few, grouped and searchable | Settings recipe with built-in search |

## One family, many apps

All Remedy apps share the token set, the type scale (Remedy Sans UI), the shape language (small radii, no heavy shadows by default) and the motion curves. The **Remedy theme set** ships as `remedy-ui-themes`: Dark Forest as the default, plus the palettes defined in RemedyAI's `themes.ts` and already shared by Portal, Hearth and FreeSpeak's skins, including the deutan, protan and tritan palettes. It is one source for every app. A user who picks a theme in one app can apply it to all of them, and to the RemedyOS desktop ([07](07-styling-and-customization.md)).
