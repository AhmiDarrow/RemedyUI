# 11. Accessibility and automation

One semantics tree serves three readers:

- screen readers and other assistive technology
- tests that find elements the way users do
- AI agents (the RemedyOS mind, the RemedyAI desktop agent) that operate apps on the user's behalf

Building it once and building it well serves all three.

## Semantics

- **Every element can carry semantics:**
  - **Role:** button, checkbox, switch, slider, text field, link, list, list item, list box and option (ARIA `listbox`/`option`: a list of choices, each selected or not, the list box single or `multiselectable`), tree item, row, cell, tab, menu, menu item, dialog, alert, status, heading (with level), image, progress, group and others.
  - **Name, description and value**, with value ranges and steps for sliders.
  - **States:** checked, selected, expanded, disabled, invalid, busy, required, read-only, protected.
  - **Relations:** labelled by, described by, controls, owns, flows to.
  - **Actions:** activate, increment, decrement, expand, collapse, scroll, set value, show menu, focus, and custom actions with labels.
- **Built-in widgets declare their semantics completely.** An app gets an accessible button by writing `button(...)`.
- **Merging.** A compound widget (a chat row with avatar, name, time and text) can merge its children into one node with a composed label: "Alex, 10:42, Meet at 10:30". Its actions (Reply, React, Copy) stay available as custom actions. A screen reader moves row by row instead of fragment by fragment. This is Flutter's semantics merging.
- **Virtual lists** report the full item count and each item's position, so "item 5,032 of 10,000" is right even though most items are not mounted.
- **Live regions.** New chat messages, transfer progress and call status changes are announced politely or assertively, with throttling so a burst does not flood the reader.
- **Protected content.** `secret_input` and other protected elements report as protected and never expose their text. Elements under a lock screen are removed from the tree.

## Platform adapters

AccessKit adapters take the tree to UI Automation on Windows, NSAccessibility on macOS, AT-SPI on Linux and the Android accessibility bridge. On RemedyOS it goes over a semantics wire. The tree is built only when a client connects, then updated incrementally from `SEMANTICS`-dirty elements each frame ([00](00-principles-and-modularity.md)).

## Beyond screen readers

- **Keyboard operability.** Every control works with the keyboard by construction ([06](06-input-and-motion.md)). CI checks that every interactive widget is reachable by Tab or arrow keys in each reference scene.
- **High contrast and forced colors.** Themes have a high-contrast variant. In Windows forced-colors mode, the engine swaps to the system's colors for text, backgrounds, buttons, links and highlights, as browsers do.
- **Text scaling.** OS text scale grows text and the layouts around it (line heights, row heights in lists) without growing icons and chrome. Layouts are tested at 100%, 150% and 200% text scale.
- **Reduced motion and reduced transparency** are honored by all built-in motion and materials.
- **Target sizes.** Interactive elements get a minimum hit target per input type: 24 dp with a mouse, 44–48 dp with touch. The hit area is extended invisibly when the visual is smaller.
- **Contrast checks** in the theme editor and in CI flag token pairs that fall below WCAG 2.2 AA, or AAA for themes that claim it.

## Testing through semantics

The headless runner finds elements as users perceive them:

```rust
let ui = TestApp::new(settings_screen());
ui.find(role(Switch).name("Noise suppression")).click();
assert!(ui.find(role(Switch).name("Noise suppression")).is_checked());
ui.find(role(Slider).name("Input gain")).set_value(150.0);
```

Tests written this way survive restyling and refactoring, and they double as accessibility checks. If a test cannot find a control by its role and name, neither can a screen reader.

## Automation by agents

Agents operate the same tree, with the user in control:

- **What an agent sees.** A snapshot of the semantics tree for windows it was granted, with each node's role, name, value, state and available actions, and the command table for the focused scope. Protected and lock-screened content is never included.
- **What an agent can do.** Invoke commands and semantic actions, such as activate a button, set a value, scroll or open a menu, through the same code paths a user's input takes. There is no synthetic mouse or keyboard injection, so agent actions are exact, cannot misclick, and run the app's own checks and undo.
- **Consent.** The user grants an agent access per app, or per window, with an expiry. Scopes are `read`, `act` and `act-with-confirmation`. The engine draws a visible indicator while an agent is connected, and the `trusted_prompt` layer ([08](08-security-and-privacy.md)) cannot be operated by an agent at all.
- **Accountability.** Every agent action is recorded in the window's undo history with the agent named ("Undo: set Input gain, by assistant"). An app can require confirmation for any command it marks `destructive`.
- **Transport.** The protocol runs over the same channel as the platform accessibility adapter where possible, or a local IPC endpoint the bridge opens only while access is granted. On RemedyOS it is the kernel's semantics wire, so the mind gets the same tree as a screen reader, under the same consent.
