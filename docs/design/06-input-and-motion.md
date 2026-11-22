# 6. Input and motion

Input and motion are where people decide whether an app feels good. The models come from:

- **Flutter:** the gesture arena.
- **Chromium:** passive listeners and input prediction.
- **iOS and Android:** scroll physics.
- **SwiftUI and Core Animation:** interruptible springs.

## Event flow

```
bridge event ─▶ coalesce (keep history) ─▶ hit test ─▶ capture phase ─▶ target ─▶ bubble phase
                                              │
                                              └─▶ gesture arena (recognizers compete) ─▶ winner's callbacks
```

- **One event model** for every platform: `PointerEvent` (mouse, touch, pen; id, position, buttons, pressure, tilt, twist, timestamp, coalesced and predicted points), `KeyEvent` (physical key, logical key, text, modifiers, repeat, IME state), `WheelEvent` (lines or precise pixels, phase, momentum), `FocusEvent`, `DropEvent`, `ImeEvent`. Bridges translate platform events into these. Nothing platform-shaped crosses into the engine.
- **Hit testing** walks layers front to back, using each layer's spatial index of chunk bounds, then exact tests for shapes:
  - It respects transforms, clips, rounded corners and `hit_region`s from canvases.
  - Elements can be marked `pointer_events(None)` (click-through) or `Bounds` (the whole box, ignoring transparent pixels).
- **Pointer capture.** A press captures the pointer to its target until release, so drags work outside the element and outside the window where the platform allows.
- **Passive by default.** Handlers do not block scrolling unless they declare `prevents_scroll()`. Scroll input can therefore go straight to the compositor (see [01](01-threads-and-frames.md)).

## Gestures

- **Recognizers**: tap, double tap, long press, drag, pan (horizontal, vertical, free), pinch, rotate, swipe, hover, plus the app's own recognizers through a trait.
- **The arena.** When a pointer goes down, every recognizer on the hit path joins that pointer's arena. A recognizer claims victory or concedes as input arrives: for example, a horizontal pan wins once movement passes the slop and is mostly horizontal. When one wins, the others are cancelled.
  - A swipe-to-reply on a chat row and the list's vertical scroll never both fire.
  - A button inside a draggable card still clicks.
- **Thresholds come from the platform**: touch slop, double-tap time, long-press delay, drag threshold. A mouse uses tighter slop than a finger.
- **Velocity tracking** uses the coalesced history (least-squares over the last 100 ms) and feeds flings and springs.

## Scrolling

- **Physics per platform.**
  - **Android:** fling decay with stretch overscroll.
  - **iOS and macOS:** rubber band with bounce.
  - **Windows:** precise touchpad scrolling, and wheel steps animated with a short ease.
  - **Linux:** follows the desktop's setting.
  - An app can pick one physics everywhere when it wants one feel.
- **Nested scrolling.** An inner scroll view consumes the delta until its edge, then hands the rest to its parent, with fling velocity carried across. A horizontal carousel inside a vertical page hands off by axis.
- **Snapping** to items or pages, with mandatory or proximity modes.
- **Scrollbars** match the platform: overlay scrollbars that fade on macOS, Android and touch, and classic draggable ones on Windows and Linux with a mouse. They always meet a minimum hit size.
- **Programmatic scrolling**: to an offset, an element or a list item, with an animated or instant curve. Focus changes scroll the newly focused element into view.
- **Keyboard scrolling**: arrows, Page Up and Down, Home and End, and Space, for the scroll view that holds focus or is under the pointer.

## Focus and keyboard

- **Focus is part of the tree.** Each element can be focusable. Tab order follows the tree, can be overridden per element, and can be scoped (`focus_scope`), so Tab stays inside a dialog.
- **Focus groups.** Arrow keys move inside toolbars, menus, radio groups, grids and lists, while Tab leaves the group. This is the roving tabindex pattern from WAI-ARIA, built in.
- **Spatial navigation.** For TVs and touch-only browsers (on by default there, through `SystemSettings::spatial_navigation`; off on desktops unless the app calls `Ui::set_spatial_navigation(true)`), an arrow the focused element does not take moves focus to the nearest focusable element in that direction, within the current focus scope, as the W3C CSS Spatial Navigation draft picks it: the distance between the closest edges, plus the distance along the axis, plus the cross-axis distance weighted (30 for left and right, 2 for up and down by default; `Ui::set_spatial_tuning` changes them), minus the cross-axis overlap. Widgets that take arrows (sliders, lists, menus, text fields, the player's keys) get them first. `El::focus_group()` keeps arrows inside a row or grid while it has something that way, lets them out at its edge, and returns focus to the child it last had when arrows bring focus back; `El::focus_wrap()` comes round at its ends instead. A gamepad's d-pad and left stick arrive as arrows, A as Enter and B as `Key::Back`, sent to the focused window as the keyboard's keys are: on Android as the system's own keys (`KEYCODE_BUTTON_A`/`_B`), on the web through the Gamepad API's standard mapping, on Windows through XInput for Xbox-class pads and Windows.Gaming.Input's `RawGameController` for every other controller (PlayStation, Switch Pro, generic HID; A and B are the south and east face buttons by the driver's labels or the vendor's layout, the first hat is the d-pad, the first two axes the stick once each has been near its center; Xbox-class pads are left to XInput so none is read twice; both looked over once a second for pads plugged in), and on Linux through evdev under `/dev/input` (inotify finds pads plugged in later; a device the user may not read is passed over, and a sandbox without `/dev/input` simply has no pads). Where the bridge reads pads itself, a held direction repeats as a held arrow key does, after the system's keyboard repeat delay and at its rate (`SystemSettings::key_repeat_delay`/`key_repeat_interval`: Windows' keyboard settings; on Linux the Wayland compositor's `repeat_info`, else XKB's controls, then GNOME's settings, then KDE's `kcminputrc`; 500 ms and 33 ms where nothing says), and the stick counts once it leans past its dead zone (0.5); `App::set_gamepad_tuning` sets the app's own values. One state machine, `remedy_ui_input::GamepadKeys`, turns readings into presses, releases and repeats on every platform; the shared winit core runs it on a thread of its own over the `PadSource` a bridge hands it (`PlatformHooks::gamepads`), and the OS calls stay in each bridge's `os` module. The new focus scrolls into view. An Escape nothing took is offered as `Key::Back`, which a `Router` or `NavStack` takes.
- **Focus-visible.** The theme draws the focus ring only after keyboard use, never after a click, as browsers do.
- **Restore.** Closing a modal, menu or popover returns focus to what opened it.
- **Shortcuts are layout-aware.** Shortcuts match the logical key in the user's layout ("Ctrl+Z" is the key labelled Z), except for physical-position shortcuts (game-style WASD), which are declared as physical.
- **Platform primary modifier.** One `Primary` modifier is Ctrl on Windows, Linux and RemedyOS, and Cmd on macOS. Apps write `Primary+S` once.
- **IME takes precedence.** While IME composition is active, shortcuts do not fire on keys the IME consumes.
- **Input purposes.** `TextField::purpose(InputPurpose)` says what a field holds: `Text`, `Digits`, `Decimal`, `Url`, `Email`, `Search`, `Username`, `CurrentPassword`, `NewPassword` or `OneTimeCode` (a secret field is `CurrentPassword` unless set). It rides on every text-input session (`TextInputState::purpose`). The web's hidden `<input>` gets `type`, `inputmode`, `autocomplete` (`username`, `current-password`, `new-password`, `one-time-code`, `email`) and `enterkeyhint` (`TextInputState::enter_key`: what `TextField::enter_key(EnterKey)` picked, else `search` for a search, `go` for an address, `enter` for several lines, `done` otherwise), so soft keyboards show the right keys and password managers fill the right fields; Android's `EditorInfo` gets the matching `inputType` and `imeOptions` (`IME_ACTION_DONE`, `GO`, `SEARCH`, `SEND`, `NEXT`, `PREVIOUS`) and the view the matching autofill hints (`username`, `emailAddress`, `password`, `newPassword`, `smsOTPCode`), so autofill services fill the field. `EnterKey::Next` and `Previous` move focus as Tab and Shift+Tab do, on the web and Android alike; Windows reports the TSF input scope (`IS_DIGITS`, `IS_URL`, `IS_PASSWORD`...) through `GUID_PROP_INPUTSCOPE`, which the touch keyboard follows; Linux (through winit's text-input-v3 and IBus support) gets the password purpose for both password purposes, the only content purposes winit carries.
- **Pointer capabilities.** `Ui::pointer_caps()` is a `Signal<PointerCaps { hover, fine }>` for hover-only affordances: on the web it follows the `(hover: hover)` and `(pointer: fine)` media queries as they change; natively it starts from the pointing hardware attached (`SystemSettings::pointer_devices`, `PointerCaps::of_devices`: a mouse or touchpad, else a touch screen, else a pen) and follows the pointer last used, so a convertible laptop folded into a tablet changes it with its first touch. Windows reads `SM_MOUSEPRESENT`, `SM_DIGITIZER`, `SM_MAXIMUMTOUCHES` and tablet posture (`SM_CONVERTIBLESLATEMODE`) and hears of HID and mouse devices coming and going through `CM_Register_Notification`; Linux reads the kernel's device list (`/proc/bus/input/devices`, the same capability bits udev goes by, on X11 and Wayland alike) and watches `/dev/input` with inotify; Android reads `InputDevice` sources (mouse, touchpad, touch screen, stylus) and listens with `InputManager.InputDeviceListener`. Each change of hardware sets the caps again; where the hardware cannot be read it starts as a mouse on desktops and a finger on phones.

## Commands

A command is the unit of an action:

```rust
Command::new("message.reply").label(t("Reply")).icon(icons::REPLY)
    .shortcut(Primary + Key::R).enabled(can_reply).run(move |cx| reply_to(selected.get()))
```

- **Scopes.** Commands are registered at app, window, screen or element scope. The innermost enabled command for a chord wins.
- **One source.** Menus, context menus, toolbars, the command palette, tooltips ("Reply (Ctrl+R)") and accessibility actions all read from the command table. One definition shows the same label, icon, shortcut and enabled state everywhere.
- **User rebinding.** The user can rebind any shortcut. Bindings are stored as editable data (see [07](07-styling-and-customization.md)), and conflicts are shown when they are set.
- **Agents act through commands.** The same table is what an agent invokes (see [11](11-accessibility-and-automation.md)), so agent actions run the same code, checks and undo as the user's own.

## Drag and drop

- **Sources** declare payloads (text, rich text, image, files, URI list, custom typed data) and allowed operations (copy, move, link). The drag preview is a rendered element, by default a snapshot of the source.
- **Targets** declare accepted types and get enter, over, leave and drop events with the offered types. The app answers which operation applies, and the pointer shows it.
- **Between apps.** The OS drag and drop protocol (OLE, NSDraggingSource, Wayland and X11 DnD, Android `DragEvent`) is handled by the bridge. Files from the OS arrive as paths or, when sandboxed, as file handles.
- **Reorder** in lists and grids is built in. Items move aside with an animation, auto-scroll near the edges, and keyboard reorder is available for accessibility.

## Motion

- **One clock.** Each window's frame clock reports the *predicted presentation time* of the frame being built, and every animation samples at that time. Motion stays smooth at 60, 120 or 240 Hz and under variable refresh.
- **Animatable values** implement `Animate` (interpolation and distance). Built in: numbers, points, sizes, rects, colors (interpolated in Oklab, so fades do not muddy), transforms, corner radii, shadows and paddings.
- **Kinds.**
  - **Tween** with an easing curve: cubic Bézier, plus standard curves named in the theme (`standard`, `emphasized`, `decelerate`, `accelerate`).
  - **Spring** with stiffness and damping, or the designer-friendly duration and bounce form. Solved analytically, not stepped, so it is exact at any frame rate.
  - **Keyframes** with per-segment easing.
  - **Decay** for flings.
- **Interruption.** A new target starts from the current presented value and velocity, so motion never jumps. Springs make this natural, and tweens convert to a spring-like handoff.
- **Where it runs.** Transform, opacity, clip-rect and scroll animations on layered elements run on the compositor thread from a description sent once. Everything else runs on the UI thread from the same clock.
- **Implicit transitions** are declared in style: `transition(Opacity, theme.motion.quick)`. Any change to that property animates.
- **Enter, exit and move.** `show()` and `for_each()` take `enter`, `exit` and `move` transitions (`Show::enter`/`exit`; `for_each_with` with `ListTransitions`; `El::flip` is the FLIP step on its own). Exiting elements are kept, inert and outside hit testing and accessibility, until their exit ends. Moves use FLIP: measure, invert, play.
- **Shared elements.** Elements tagged with the same `shared_id` on two screens morph between them during navigation.
- **Gesture-driven.** A pan's translation drives a value directly. On release, a spring takes over with the pan's velocity toward the snap target. This covers drawers, sheets, swipe actions and pull to refresh.
- **In code.** `El::animate(Target::Opacity(0.0), motion)` moves one property (opacity, colors, border, radius, padding, or the drawn offset and scale) on the window's frame clock; `animate_then` calls back when it arrives, and `animate_style` moves what a style patch sets. `Ui::motion(Pace::Standard)` gives the theme's spring. Painters read the frame time from `PaintCx::time` and call `request_next_frame` while they move, so looping animations (spinners, indeterminate progress) need no timers. The runner sets the clock to each frame's presentation time; tests set it with `Ui::set_frame_time`. `remedy_ui::motion` gathers the motion types.
- **Reduced motion.** When the OS or app asks for it, transitions become instant or a short cross-fade, parallax and auto-playing animations stop, and springs settle instantly. Nothing breaks, because state never depends on an animation finishing.
- **Battery.** Animations on hidden or covered windows, off-screen elements or a battery-saver device stop requesting frames. Looping decorative animations (spinners, shimmer) pause when nothing else is happening on screen for a while.
