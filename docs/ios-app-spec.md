# Baby Burrito for iOS: product and design spec

Status: spec, written 2026-10-09. Companion to [ios-cloudkit-plan.md](ios-cloudkit-plan.md), which covers sync, sharing and the Live Activity internals. This document covers what the app is, how it looks and feels, and the budgets it must hit.

The app uses **no backend**. All data lives on the device and in the parents' own iCloud (CloudKit). The Rust service in this repo is not used.

## In one sentence

Log a feeding in one tap, at 3 a.m., with one hand, and always know when the next one is due.

## Principles

1. **One tap for the common case.** Most feedings repeat the previous one. Repeating it must take one tap, from the Lock Screen, Dynamic Island, Action button or the app.
2. **One screen.** Everything daily lives on Today. Trends and Settings are one push away. No tab bar, no onboarding tour, no accounts.
3. **Lively, not loud.** Motion, haptics and color confirm every action. Nothing blinks, nags or notifies.
4. **Native or nothing.** System components only, so the iOS 27 design comes for free and stays correct when Apple tunes it.
5. **Small and fast.** No third-party code, no images beyond the app icon, no network on the launch path.

## Platform target

|                  |                                                                                                                                                                                      |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| OS               | **iOS 27 only.** Deployment target 27.0. No availability checks, no fallbacks                                                                                                        |
| Reference device | **iPhone 18 Pro**: 6.3", 2622 × 1206 px at 460 ppi (**402 × 874 pt @3x**), ProMotion up to 120 Hz, Always-On display, smaller Dynamic Island, Action button, Camera Control, A20 Pro |
| Other devices    | Any iPhone that runs iOS 27 must work correctly. Layouts are tuned and screenshotted on the 18 Pro first                                                                             |
| Not supported    | iPad, Mac, Apple Watch, landscape (portrait only)                                                                                                                                    |
| Languages        | English first. All strings in a String Catalog from day one, so adding a language is translation only                                                                                |
| Toolchain        | The Xcode that ships the iOS 27 SDK, Swift 6 language mode with strict concurrency                                                                                                   |

### iPhone 18 Pro specifics to design for

- **Smaller Dynamic Island.** Face ID's infrared camera moved under the display, so the pill is narrower. Compact Live Activity views get less width than on the 17 Pro. Design compact content for the narrower width and test on device. Reports say the island can show more simultaneous Live Activities; the app must look right when it isn't the primary one (minimal view).
- **Always-On display.** The Lock Screen Live Activity is visible all night on the nightstand. It must look good dimmed (see Live Activity).
- **120 Hz.** Scrolling and transitions must hold 120 fps. No layout work inside timers.
- **Action button.** Can run an App Shortcut. "Log feeding" is offered as one (see System surfaces).

## Design language: Liquid Glass on iOS 27

iOS 27 keeps Liquid Glass but refines it: default transparency is lower, the glass material itself was revised for legibility, and users get a system slider from clearer to more tinted glass. Search returns into the tab bar. What this means for us:

- **Use system components and let the system draw the glass.** `NavigationStack`, toolbars, sheets, menus, `Button` styles and the title menu all pick up the current material, the user's transparency setting, Reduce Transparency and Increase Contrast automatically. **Never draw custom blur or hard-code a material.** Custom glass, if ever needed, uses `.glassEffect()` inside a `GlassEffectContainer`, nothing else.
- **Glass is for controls, not content.** Content (the countdown, the feed list, charts) sits on the plain system background. Controls float above it in glass: the bottom quick-log bar, toolbar buttons and sheets.
- **One accent, three kind colors.** The accent tints prominent glass buttons. Each feeding kind has a color used for its symbol, chart series and chips. The colors are warm and calm, never red, because a baby app shouldn't feel like an alarm.

  | Kind                 | Color | Symbol (SF Symbols, final pick at design time) |
  | -------------------- | ----- | ---------------------------------------------- |
  | Nursing              | Rose  | custom template symbol, fallback `heart.fill`  |
  | Breast milk (bottle) | Amber | `drop.fill`                                    |
  | Formula (bottle)     | Teal  | `waterbottle.fill`                             |

- **Type.** SF Pro with `.fontDesign(.rounded)` for all numbers (countdown, ml, minutes) and `.monospacedDigit()` wherever digits tick. Large, friendly numerals are the main visual element.
- **Concentric shapes.** Cards and buttons use the system's concentric corner radii (`ConcentricRectangle` / container-relative shapes) so they nest cleanly inside the rounded display and sheets.
- **Light and dark.** Follow the system appearance. Dark mode is the night mode: no pure white surfaces, kind colors desaturated slightly.
- **Icon.** A layered Liquid Glass app icon made in Icon Composer (a swaddled baby burrito). It must read in light, dark, tinted and clear modes.

**Verify at implementation time** against the iOS 27 Human Interface Guidelines and SwiftUI release notes. The glass APIs named here are the iOS 26 ones; iOS 27 changes reported so far are visual tuning that system components inherit.

## Making it lively

| Moment         | Feedback                                                                                                                            |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Feeding logged | `.sensoryFeedback(.success)`, the kind symbol does a `.bounce` symbol effect, totals roll with `.contentTransition(.numericText())` |
| Countdown      | System-rendered `Text(timerInterval:)` and a progress ring that fills smoothly. No app timers                                       |
| Feeding due    | Ring completes, the label changes to "Due now" with a gentle `.pulse` on the symbol. Then it counts up softly ("12 min ago")        |
| Undo           | Spring the row back out, `.sensoryFeedback(.impact(weight: .light))`                                                                |
| Side picker    | Picking left/right animates a small highlight between the two halves, like a segmented control                                      |
| Empty state    | The app icon's burrito in a `.breathe` animation and one button: "Log the first feeding"                                            |

Copy is short and warm: "Burrito ate 90 ml", "Next in 1 h 20 min", "Due now". No exclamation marks at night.

All motion respects Reduce Motion (springs become cross-fades, symbol effects are skipped).

## Screens

Wireframes are at the reference width of 402 pt. One `NavigationStack`, with **Today** as its root.

### Today

```
┌──────────────────────────────────────────┐
│ Burrito ⌄                         (⚙︎)   │ title menu switches baby; glass toolbar button
│                                          │
│  NEXT FEEDING                            │
│  in 1:23:10                      ◔       │ large rounded numerals, progress ring
│  at 17:05 · every 3 h                    │ interval tappable → edit
│                                          │
│  ┌ LAST ──────────────────────────────┐  │
│  │ ♥ Nursing · left · 15 min          │  │
│  │ 14:05 · 2 h 10 min ago · Mom       │  │
│  └────────────────────────────────────┘  │
│                                          │
│  LAST 24 H                        (📊)   │ pushes Trends
│  540 ml   6 bottles   45 min nursing     │
│  ▁▃ ▅ ▂▇ ▃  ▅  ▂ ▃ ▅   (24 h timeline)   │ dots/bars in kind colors
│                                          │
│  TODAY                                   │
│  17:05  💧 Breast milk      90 ml        │ swipe: edit / delete
│  14:05  ♥ Nursing      L · 15 min        │
│  11:10  🍼 Formula          80 ml        │
│  YESTERDAY                               │
│  …                                       │ lazy, grouped by local day, endless
│                                          │
│ ╭──────────────────────────────╮ ╭────╮  │ bottom toolbar, glass
│ │ ↻ Right · 15 min             │ │ ＋ │  │ repeat-last  |  log sheet
│ ╰──────────────────────────────╯ ╰────╯  │
└──────────────────────────────────────────┘
```

- **Repeat button** (bottom left, wide, `.glassProminent`): shows exactly what it will log, following the plan's repeat rule: bottles repeat kind and amount; nursing repeats the duration on the other breast with `fedAt = now − duration`. One tap logs and shows the confirmation (below). Hidden for viewers.
- **＋ button**: opens the Log sheet prefilled from the last feeding.
- **History is the same list**: Today scrolls back through all days. There is no separate History screen.
- **Title menu**: lists babies (own and shared) when there is more than one, plus "Add baby". Switching changes Today and the Live Activity to that baby.
- **Viewer role**: the bottom toolbar is replaced by a quiet "Shared by Mom · view only" label. Swipe actions are off.

### Log sheet

A sheet at the medium detent, expandable to large. Prefilled from the last feeding. Also used to edit an existing feeding.

```
╭──────────────────────────────────────────╮
│  Log feeding                       Done  │
│  ( Nursing | Breast milk | Formula )     │ segmented, kind colors
│                                          │
│  Nursing:                                │
│   ┌──── Left ────┐ ┌──── Right ───┐      │ big halves, 64 pt tall
│   └──────────────┘ └──────────────┘      │
│              ( Both )                    │
│   Duration   5  10 [15] 20  30   − 15 +  │ chips + stepper (1–120)
│                                          │
│  Bottle:                                 │
│   Amount    30  60 [90] 120  150  − 90 + │ chips + stepper by 10 (1–500)
│                                          │
│  Started    Today 14:05             ⌄    │ compact DatePicker
╰──────────────────────────────────────────╯
```

- Chips come from the baby's recent history (the five most common values), not fixed lists, so the common amount is always one tap.
- Validation is live and mirrors the Domain rules; **Done** is disabled with an inline reason when invalid. The fields of the other kind are never shown, so they can't be invalid.
- `fedAt` defaults to now for bottles and to now − duration for nursing, recalculated when the duration changes until the user touches the time.

### Logged confirmation

Shown after any one-tap log (Repeat button, Live Activity tap, Action button). It is the Log sheet in a "logged" state:

- Title "Logged 17:05" with the kind symbol bouncing, and a large **Undo** button.
- The fields are live: changing them edits **that same feeding** (never a duplicate).
- Dismisses itself after 8 seconds with no interaction, so a parent holding a baby can just put the phone down.

### Trends

Pushed from Today.

- Range picker: 7 days / 30 days.
- **Bottles**: daily stacked bars of ml by kind, with the daily average as a rule mark.
- **Nursing**: daily bars of minutes split by side (left / right / both), so imbalance is visible.
- **Rhythm**: average and longest interval per day, start to start.
- Tapping a day scrolls Today's list to that day.
- Charts carry `accessibilityChartDescriptor` so VoiceOver users get audio graphs.

### Settings

Pushed from the toolbar gear. A plain grouped `Form`.

- **Baby**: name, birth date (optional), feeding interval (stepper in 15-minute steps, 15 min–12 h, plus chips 2 h, 2½ h, 3 h, 3½ h, 4 h). Editable by the owner and editors; it syncs, so both parents count down to the same time.
- **Sharing**: invite (system share sheet), participants and their roles, stop sharing / leave.
- **Lock Screen**: "Show on Lock Screen" toggle with a live preview of the activity. Explains how to re-enable Live Activities if the system has them off.
- **iCloud status**: a single row, only shown when something needs attention.
- **Delete all data**: deletes this baby's zone and the local store, with a confirmation.

### First run

Two steps, no tour:

1. Baby's name and feeding interval (default 3 h), with a "Joining your partner's log? Open their invite link" note for the second parent.
2. "Show on Lock Screen" with the preview and one button to turn it on.

Then Today, in its empty state.

## System surfaces

All of them do the same two things: show last and next feeding, and log a repeat of the last feeding by opening the app (see the plan for why logging always happens in the foreground app).

| Surface                                                 | Content                                                                                                                      | Tap                                    |
| ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| **Live Activity, Lock Screen**                          | Baby name, "Next in 1:23:10" countdown and progress, last feeding line, who logged it                                        | Opens app → logs repeat → confirmation |
| **Live Activity, Dynamic Island**                       | Compact: kind symbol + short countdown (`1:23`). Minimal: progress ring. Expanded: same as Lock Screen                       | Same                                   |
| **Always-On display**                                   | Same Lock Screen view; under `isLuminanceReduced` the progress fill and kind colors go monochrome and the seconds are hidden | n/a                                    |
| **StandBy** (phone charging on its side at the bedside) | The Lock Screen view, scaled up by the system. Must be legible from across the room: the countdown is the largest element    | Same                                   |
| **Action button**                                       | App Shortcut "Log feeding" (`AppIntent` with `openAppWhenRun`)                                                               | Same flow as a Live Activity tap       |
| **Control Center**                                      | A control "Log feeding" with the same intent                                                                                 | Same                                   |
| **Home Screen widget** (small, medium)                  | Countdown + last feeding, using the Live Activity views                                                                      | Same deep link                         |
| **Siri / Shortcuts**                                    | "Log 90 ml formula", "When is the next feeding?" via App Intents and App Shortcuts                                           | —                                      |

There are **no notifications**. The Live Activity replaces them.

Viewers (read-only participants) get every display surface, and taps only open the app.

## Data and storage

Defined in [ios-cloudkit-plan.md](ios-cloudkit-plan.md); summary:

- **Storage:** CloudKit. One record zone per baby in the owner's private database, shared as a whole with the other parent through a zone-wide `CKShare` (read-only or read-write). All fields are `encryptedValues`, end-to-end encrypted.
- **Local:** SwiftData store (`cloudKitDatabase: .none`) in an App Group, so the app reads and writes locally first and the widget extension can read it. `CKSyncEngine` syncs it, one engine per database.
- **Records:** `Baby` (name, birth date, feeding interval) and `Feeding` (start time, kind, ml or duration + side).
- **Offline:** everything works offline; changes sync later.

## Budgets

Measured on an iPhone 18 Pro, release build. They are acceptance criteria, not goals.

| Budget                                                  | Limit                           | How it's kept                                                                                      |
| ------------------------------------------------------- | ------------------------------- | -------------------------------------------------------------------------------------------------- |
| App Store download size                                 | ≤ 5 MB (hard limit 10 MB)       | No dependencies, no bundled images or fonts (SF Symbols, system fonts), one Icon Composer icon     |
| Cold launch to Today interactive                        | ≤ 400 ms                        | No network, no CloudKit work before first frame; sync starts after                                 |
| Warm launch / Live Activity tap to confirmation visible | ≤ 150 ms after unlock           | The deep link handler writes one row locally, then renders. Sync and activity restart happen after |
| Live Activity restarted after a log                     | ≤ 1 s                           |                                                                                                    |
| Scrolling Today with 2 years of data (~7,000 feedings)  | 120 fps, no hitches             | Lazy list, `@Query` with fetch limits per section, stats computed off the main actor               |
| Memory, steady state                                    | ≤ 50 MB                         |                                                                                                    |
| Background work                                         | CloudKit silent pushes only     | No background refresh tasks, no location, no timers                                                |
| Battery while the Live Activity runs                    | Not measurable against baseline | Countdown is system-rendered; the activity is only updated when data changes                       |

### Code constraints that keep it small

- **No third-party dependencies.** Apple frameworks only: SwiftUI, SwiftData, CloudKit, ActivityKit, WidgetKit, AppIntents, Charts.
- **One local Swift package, `BabyCore`** (Domain, Persistence, Sync, Activity content), shared by the app and the widget extension so code isn't duplicated in two binaries.
- **No custom fonts, images, Lottie or analytics.** Animations are SwiftUI and SF Symbol effects.
- **Pure `Domain`.** Validation and statistics are framework-free Swift, ported from `src/feedings/mod.rs` and `src/stats.rs` with the same test cases.

## Accessibility

- **Dynamic Type:** every size up to AX5. At accessibility sizes the Today header stacks vertically and the bottom toolbar keeps only the Repeat button with the ＋ moved to the toolbar.
- **VoiceOver:** the countdown reads as "Next feeding in 1 hour 23 minutes, at 5:05 PM". Feeding rows read as one element ("Nursing, left breast, 15 minutes, 2:05 PM, logged by Mom"), with edit and delete as custom actions.
- **Targets:** primary actions at least 60 pt tall, all in the bottom half of the screen for one-handed use while holding a baby.
- **Contrast and transparency:** handled by system components (Increase Contrast, Reduce Transparency, the iOS 27 glass slider). Kind colors meet 4.5:1 on both backgrounds for text uses.
- **Color is never the only signal:** every kind also has its symbol and name.

## Privacy

- No accounts, no server, no analytics, no crash reporting SDKs. The developer never receives any data.
- Data lives in the user's iCloud, end-to-end encrypted, and is shared only with participants the owner invites.
- Expected App Store privacy label: **Data Not Collected** (confirm at submission).
- Privacy manifest (`PrivacyInfo.xcprivacy`) declares no tracking and only the required-reason APIs actually used.

## Non-goals (v1)

- iPad, Mac, Apple Watch apps. (A Watch app is the most likely v2.)
- Notifications of any kind.
- Diapers, sleep, growth, pumping, medications. Feedings only.
- Units other than ml.
- Export. Possibly v2 as a CSV via the share sheet.
- Nursing timer (decided: after v1).
- Any server, including a push relay (see the plan's trade-off).

## Acceptance (v1 is done when)

- [ ] A one-tap repeat from the Lock Screen, the Dynamic Island, the Action button and the Repeat button each logs exactly one correct feeding and shows the confirmation with Undo.
- [ ] The Log sheet logs nursing (side + duration) and bottles (kind + ml) and rejects invalid input inline.
- [ ] Both parents see the same feedings and the same next feeding time, with the second parent as viewer and as editor.
- [ ] The Live Activity survives a full night of feedings, is legible on the Always-On display and in StandBy, and looks right in the 18 Pro's smaller Dynamic Island.
- [ ] Every budget in the table above is met on an iPhone 18 Pro.
- [ ] Works fully offline, and syncs when back online.
- [ ] VoiceOver and AX5 Dynamic Type walkthroughs of Today, Log and Trends pass.

## Decisions made

- **Nursing timer:** not in v1. Revisit after v1 is in daily use. It needs no schema change, since a feeding is only written when the timer stops.
- **Repo:** the app lives in its own repo, `bridzius/baby-burrito-ios`. This repo (`baby-burrito-back`) keeps the Rust fallback and these docs.
- **Baby switcher:** the title menu on Today is enough. The Live Activity shows the selected baby only.

## Sources

- [iPhone 18 Pro tech specs (Apple)](https://support.apple.com/en-us/148590)
- [iPhone 18 Pro: smaller Dynamic Island, Face ID IR camera under the display (MacRumors)](https://www.macrumors.com/roundup/iphone-18-pro/)
- [Liquid Glass, including the iOS 27 changes (Wikipedia)](https://en.wikipedia.org/wiki/Liquid_Glass)
- [iOS 27 Liquid Glass transparency control (BGR)](https://www.bgr.com/2191219/ios-27-liquid-glass-fix-customization/)
- [iOS 27 Liquid Glass refinements for developers (TechTimes)](https://www.techtimes.com/articles/317975/20260608/apple-liquid-glass-ios-27-wwdc-2026-brings-refinements-developers-must-adopt-today.htm)
- [WWDC26 SwiftData group lab (no SwiftData sharing announced)](https://developer.apple.com/videos/play/wwdc2026/8017/)
