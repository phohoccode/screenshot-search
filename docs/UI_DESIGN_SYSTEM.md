# Screenshot Search — UI Design System

## 1. Design Direction

The application uses a **Minimal SaaS / shadcn-style desktop UI**.

Primary references:

- shadcn/ui visual language
- Radix-style interaction patterns
- modern SaaS dashboards
- restrained desktop productivity tools

The goal is not to visually clone any product.

The goal is to maintain:

- clean hierarchy
- neutral surfaces
- compact controls
- strong readability
- subtle borders
- minimal decoration
- predictable component behavior
- consistent spacing
- accessible interaction states

The application should feel like a polished modern SaaS product adapted to a native desktop utility.

---

## 2. Core Visual Principles

### Minimal

Prefer the simplest visual solution that communicates hierarchy.

Avoid decorative UI that does not improve usability.

### Neutral

Use a mostly neutral color system.

Color should be used intentionally for:

- primary actions
- active/focused state
- success
- warning
- destructive actions
- semantic statuses

Do not use multiple unrelated accent colors.

### Compact

This is a desktop productivity application.

Avoid oversized:

- buttons
- cards
- headings
- empty states
- paddings
- dialogs

Controls should feel efficient rather than mobile-first.

### Structured

Use:

- borders
- spacing
- typography
- background contrast

to create hierarchy.

Do not rely on heavy shadows or gradients to separate sections.

---

## 3. Theme Architecture

Use CSS variables/tokens instead of hardcoded colors in components.

Recommended semantic tokens:

```css
--background
--foreground

--card
--card-foreground

--popover
--popover-foreground

--primary
--primary-foreground

--secondary
--secondary-foreground

--muted
--muted-foreground

--accent
--accent-foreground

--destructive
--destructive-foreground

--border
--input
--ring
```

Prefer a shadcn-compatible token structure.

Components should consume semantic classes/tokens such as:

```text
bg-background
text-foreground
bg-card
text-muted-foreground
border-border
bg-primary
text-primary-foreground
```

Avoid arbitrary one-off color values.

---

## 4. Light Theme

Default direction:

- soft white/neutral page background
- white or slightly elevated card surfaces
- dark neutral text
- subtle gray borders
- restrained primary accent

Visual target:

```text
Background       light neutral
Surface          white / near-white
Border           subtle neutral
Primary text     high contrast
Secondary text   muted neutral
Accent           single restrained brand color
```

Do not use pure black extensively for normal body text if a softer foreground token is available.

---

## 5. Dark Theme

Dark mode should be designed, not produced by naive inversion.

Direction:

- near-black neutral background
- slightly lighter elevated surfaces
- subtle borders
- off-white foreground
- muted gray secondary text
- same semantic accent family

Avoid:

- glowing borders
- excessive neon accents
- large bright surfaces
- pure black-on-pure-white contrast where softer tokens work better

---

## 6. Typography

Use one primary sans-serif UI font stack.

Recommended:

```css
font-family:
  Inter,
  ui-sans-serif,
  system-ui,
  -apple-system,
  BlinkMacSystemFont,
  "Segoe UI",
  sans-serif;
```

Do not bundle external font files unless intentionally approved.

### Hierarchy

Suggested desktop scale:

```text
Page title        24–28px / semibold
Section title     16–18px / semibold
Card title        14–16px / medium or semibold
Body              14px
Secondary text    12–13px
Metadata          12px
```

Avoid excessive font-size variation.

Use font weight and spacing before introducing a new size.

### Text Rules

- Body copy should generally be 14px.
- Dense metadata can use 12px.
- Avoid long all-caps labels.
- Prefer sentence case.
- Truncate filenames when necessary but preserve access to the full value via tooltip or detail view.
- Long OCR text must wrap safely.

---

## 7. Spacing System

Use a consistent 4px-based scale.

Preferred spacing:

```text
4px
8px
12px
16px
20px
24px
32px
40px
```

Common rules:

```text
Button internal gap        8px
Input icon gap             8px
Small component gap        8px
Card internal spacing      12–16px
Section gap                24px
Page padding               20–24px desktop
Dialog padding             20–24px
```

Avoid random spacing such as:

```text
13px
17px
19px
27px
```

unless required by a measured layout constraint.

---

## 8. Border Radius

Use restrained shadcn-style rounding.

Recommended:

```text
Small controls       6px
Buttons / inputs     6–8px
Cards                8–10px
Dialogs              10–12px
Large preview        10–12px
```

Avoid:

- excessive pill-shaped controls
- 20–32px rounded cards everywhere
- nested rounded containers without purpose

Use fully rounded pills only for appropriate elements such as:

- compact status badges
- tags
- segmented chips

---

## 9. Borders

Borders are a primary structural tool.

Preferred:

```text
1px solid var(--border)
```

Use borders for:

- cards
- inputs
- sidebars
- separators
- table rows
- dialogs
- selected screenshot outlines

Avoid thick borders unless used for explicit state.

---

## 10. Shadows

Use shadows sparingly.

Cards should usually rely on border + surface contrast rather than shadow.

Allowed use cases:

- dialogs
- popovers
- dropdowns
- floating command/search surfaces
- elevated preview overlays

Shadows should be subtle.

Avoid:

- deep marketing-style shadows
- colored shadows
- glow effects

---

## 11. Icons

Use a single icon system.

Preferred:

```text
lucide-react
```

Rules:

- common icon sizes: 14, 16, 18, 20px
- buttons usually use 16px icons
- sidebar/navigation icons usually 16–18px
- avoid mixing multiple icon libraries
- icon-only actions require tooltip or accessible label
- do not use emoji as functional UI icons

Icons should support the label, not replace clarity.

---

## 12. Component Strategy

Prefer reusable shadcn-style components.

Suggested primitives:

```text
Button
Input
Textarea
Label
Card
Badge
Separator
Tooltip
DropdownMenu
ContextMenu
Dialog
AlertDialog
Sheet
Popover
Command
ScrollArea
Progress
Skeleton
Tabs
Switch
Checkbox
Select
Toast
```

Do not create a custom implementation when an existing project component already solves the same interaction correctly.

Before building a new UI primitive:

1. search existing UI components;
2. check whether a shadcn/Radix-style primitive is already present;
3. reuse or extend it;
4. custom-build only when necessary.

---

## 13. Buttons

### Variants

Recommended variants:

```text
default
secondary
outline
ghost
destructive
link
```

### Sizes

Recommended:

```text
sm
default
icon
```

Desktop controls should remain compact.

Avoid giant CTA buttons inside utility screens.

### Rules

Primary action:

- at most one visually dominant action per local context
- use `default` variant

Secondary actions:

- outline, secondary, or ghost

Dangerous actions:

- destructive
- require confirmation when irreversible

Icon-only buttons:

- square
- use tooltip
- accessible name required

Do not place five equally prominent buttons in one row.

---

## 14. Inputs and Search

The search input is the primary interaction of Screenshot Search.

### Main Search

The main search field should:

- be visually prominent without being oversized
- have a search icon
- support keyboard focus immediately
- show a clear placeholder
- support clear/reset action when text exists
- use visible focus ring
- remain consistent between empty/results states

Example direction:

```text
[ Search icon ] Search screenshots...                       [⌘K]
```

Do not turn the main search field into a huge hero marketing element.

### Standard Inputs

Use:

- consistent height
- subtle border
- neutral background
- clear placeholder
- visible focus ring
- validation message below

---

## 15. Cards

Cards are not the default container for everything.

Use cards only when a group benefits from a bounded surface.

Screenshot result cards may contain:

```text
thumbnail
filename
date/time
small match snippet
optional source/folder metadata
```

Rules:

- compact
- border-based
- subtle hover state
- no excessive padding
- avoid nested cards

Selected cards should use:

- border/ring state
- subtle background change

not dramatic scaling.

---

## 16. Screenshot Grid

The screenshot grid is a core interface.

Requirements:

- responsive columns based on available width
- consistent thumbnail aspect behavior
- lazy loading
- clear hover state
- selected state
- filename truncation
- optional matched snippet
- no horizontal overflow

Prefer CSS Grid.

Example:

```text
min card width: ~220–260px
gap: 12–16px
```

Do not hardcode a fixed number of columns for all window sizes.

---

## 17. Screenshot Preview

Clicking a screenshot should open a focused preview.

Preferred behavior:

- centered dialog or dedicated preview pane
- large image area
- constrained to viewport
- preserve aspect ratio
- metadata/actions in a compact side or footer region

Actions may include:

```text
Open original
Reveal in Explorer
Copy OCR text
Close
```

Future:

```text
View extracted text
Search inside OCR
AI explain
```

Do not navigate away from search results unless there is a strong UX reason.

---

## 18. Sidebar / Navigation

Keep navigation minimal.

Potential sections:

```text
Search
Folders
Indexing
Settings
```

Avoid a complex SaaS admin-style sidebar if the application does not need it.

Sidebar characteristics:

- compact width
- subtle right border
- neutral background
- clear active state
- consistent icon size
- no decorative gradients

Active state should use:

- muted/accent surface
- stronger text
- optional icon emphasis

Avoid large rounded navigation pills unless visually justified.

---

## 19. Header / Top Bar

Prefer a compact application header.

Potential contents:

```text
page title
indexing status
theme/menu actions
settings shortcut
```

Do not build a large website-style navigation bar.

---

## 20. Dialogs

Use dialogs for:

- folder configuration
- confirmations
- screenshot preview
- focused settings flows

Rules:

- concise title
- optional short description
- clear action hierarchy
- escape closes when safe
- clicking overlay closes when safe
- irreversible actions require explicit confirmation

Do not use browser-native:

```text
alert()
confirm()
prompt()
```

for product UI.

Use shared Dialog / AlertDialog components.

---

## 21. Dropdowns, Popovers, Context Menus

Use compact Radix/shadcn interaction patterns.

Good use cases:

- screenshot actions
- sort options
- filters
- secondary settings

Avoid turning every action into an always-visible button.

Context menus can be useful for screenshot cards.

---

## 22. Badges and Status

Badges should be compact and semantic.

Possible statuses:

```text
Indexed
Pending
Processing
Failed
Paused
```

Use muted styles for neutral statuses.

Reserve strong colors for meaningful state.

Do not show raw backend enum names when a user-facing label is clearer.

Example:

```text
PROCESSING
```

should render as:

```text
Processing
```

or localized equivalent.

---

## 23. Progress and Indexing UI

Long-running indexing must expose progress.

Recommended pattern:

```text
Indexing screenshots
2,348 / 8,129
[progress bar]

Paused / Cancel / Resume
```

Use a compact status area rather than a full-screen blocking modal.

User should be able to continue searching already indexed screenshots while background indexing continues.

---

## 24. Empty States

Empty states should be simple.

Example initial state:

```text
No screenshot folders yet.

Choose a folder to start indexing screenshots.

[Choose folder]
```

Search empty result:

```text
No screenshots match "prisma timeout".

Try another keyword or clear filters.
```

Avoid:

- giant illustrations
- excessive marketing copy
- oversized icons
- unnecessary gradients

---

## 25. Loading States

Use skeletons for content surfaces when useful.

Use spinner only for short, focused actions.

For long jobs use progress indicators.

Avoid blocking the entire application for background work.

---

## 26. Error States

Errors must be:

- concise
- actionable
- non-technical by default

Example:

```text
Could not read this screenshot.

The file may have been moved, deleted, or is not accessible.
```

Optional detail can expose an error code.

Do not dump raw Rust/SQLite stack traces into normal UI.

---

## 27. Toasts

Use toast notifications for short-lived feedback such as:

```text
Folder added
OCR text copied
Screenshot revealed in Explorer
Settings saved
```

Do not use toast as the only surface for critical errors requiring action.

Avoid excessive notifications during bulk indexing.

Never show one toast per screenshot.

---

## 28. Hover, Focus, Active States

Every interactive control must have:

- hover state
- focus-visible state
- active/pressed state where appropriate
- disabled state

Keyboard focus must be clearly visible.

Do not remove outlines without adding an accessible replacement.

---

## 29. Motion

Motion should be restrained.

Allowed:

- 100–200ms hover/fade transitions
- dialog/popover entrance
- subtle selection transitions

Avoid:

- bouncing
- large scale animations
- parallax
- prolonged transitions
- decorative motion

Respect reduced-motion preferences.

---

## 30. Accessibility

Minimum requirements:

- keyboard navigable controls
- visible focus state
- semantic HTML where applicable
- accessible labels
- icon-only controls have `aria-label`
- sufficient text contrast
- dialogs trap focus correctly
- escape behavior is predictable
- disabled controls remain understandable

Do not encode state using color alone.

---

## 31. Responsive Desktop Behavior

The primary target is desktop, but the window may be narrow.

UI must work across:

```text
~800px wide compact desktop window
through
large desktop monitors
```

Requirements:

- no horizontal overflow
- grid adapts
- sidebar can collapse if necessary
- dialogs stay within viewport
- preview image uses max dimensions
- toolbars wrap or collapse secondary actions

Do not optimize the app like a mobile website unless mobile support becomes an explicit requirement.

---

## 32. Density

Default density: compact to medium.

Preferred control height:

```text
Small button        ~32px
Default button      ~36px
Input               ~36–40px
Compact row         ~36–40px
```

Avoid 48–56px mobile-style controls across desktop screens.

---

## 33. Layout Rules

Prefer:

```text
App shell
├── optional compact sidebar
└── main
    ├── header/search controls
    ├── filters/status
    └── content
```

Keep content width fluid for screenshot grids.

Do not constrain screenshot results to a narrow marketing-site `max-width`.

Settings pages may use a narrower readable content width.

---

## 34. Search-First Home Screen

The main screen should prioritize:

1. search
2. screenshot results
3. filters
4. indexing state
5. navigation

Do not prioritize:

- marketing banners
- statistics dashboards
- charts
- decorative hero sections

This is a utility application, not a SaaS landing page.

---

## 35. Filters

Filters should be compact.

Possible filters:

```text
Date
Folder
File type
Sort
```

Prefer:

- dropdowns
- popovers
- command-style selectors

Avoid large filter cards consuming vertical space.

Active filters should be obvious and easy to clear.

---

## 36. Settings UI

Use grouped settings sections.

Example:

```text
General
  Start with Windows
  Theme

Screenshot folders
  ...

Indexing
  OCR language
  Resource usage

Privacy
  Telemetry
```

Rows should have:

```text
label
short description
control
```

Avoid a dashboard-like settings experience.

---

## 37. Destructive Actions

Examples:

- remove folder index
- reset database
- clear all thumbnails
- rebuild index

Use AlertDialog.

Clearly describe whether original screenshots are affected.

Example:

```text
Clear local index?

This removes OCR/search data and thumbnails.
Your original screenshots will not be deleted.
```

---

## 38. Localization

UI copy should be centralized enough to support future localization.

Do not build layouts that depend on very short English labels.

Buttons and fields should tolerate longer translated strings.

---

## 39. UI Anti-Patterns — Do Not Use

Unless explicitly requested, avoid:

- glassmorphism
- neon gradients
- gradient borders
- glowing elements
- giant rounded cards
- excessive pill UI
- excessive shadows
- dashboard charts without product need
- decorative blobs
- hero sections inside app screens
- emoji as icons
- inconsistent icon packs
- arbitrary colors
- custom controls that duplicate existing primitives
- browser `alert/confirm/prompt`
- full-screen loading for background indexing
- cards nested inside cards inside cards
- huge whitespace that lowers desktop productivity
- visually heavy sidebars
- random border-radius values

---

## 40. Component File Organization

Suggested frontend organization:

```text
src/
├── components/
│   ├── ui/               # shared shadcn-style primitives
│   ├── layout/
│   └── common/
│
├── features/
│   ├── search/
│   ├── screenshots/
│   ├── folders/
│   ├── indexing/
│   └── settings/
```

Rules:

- generic primitives belong in `components/ui`
- domain-specific components belong inside feature folders
- do not place business logic inside primitive UI components

---

## 41. `cn()` Utility

Use one class merging helper, typically:

```ts
cn(...)
```

for:

- conditional Tailwind classes
- variant composition
- className overrides

Avoid repeated manual string concatenation.

---

## 42. Variant Management

For reusable component variants, prefer a consistent variant strategy such as:

- class-variance-authority (CVA), if already part of the stack
- equivalent lightweight approach

Do not create multiple unrelated variant systems.

---

## 43. Tailwind Rules

Prefer design tokens and established utilities.

Avoid arbitrary values unless there is a real layout need.

Prefer:

```text
gap-3
p-4
h-9
rounded-md
border-border
text-muted-foreground
```

over repeated values such as:

```text
gap-[13px]
p-[17px]
rounded-[11px]
```

Arbitrary values are allowed only when standard scale values cannot express a necessary layout.

---

## 44. UI Review Checklist

Before considering a frontend task complete, check:

### Consistency

- Does it follow this design system?
- Does it reuse existing components?
- Are icon sizes consistent?
- Are radius and spacing consistent?

### Hierarchy

- Is the primary action obvious?
- Is secondary information visually secondary?
- Are there too many bordered containers?

### Desktop Usability

- Is the layout compact enough?
- Does it work in narrow desktop windows?
- Is there horizontal overflow?

### Accessibility

- Keyboard navigation works?
- Focus visible?
- Tooltips/labels for icon buttons?
- Contrast sufficient?

### States

- loading
- empty
- error
- disabled
- selected
- hover
- focus

### Privacy

- Is sensitive OCR data unnecessarily displayed?
- Is private content being included in logs or telemetry?

---

## 45. AI Coding Agent Rule

For every task that creates or modifies frontend UI, the coding agent MUST read this file before editing.

When existing UI conflicts with this file:

1. inspect the surrounding design language;
2. determine whether the existing UI or this document is stale;
3. preserve consistency with the application as a whole;
4. do not perform an unrelated full redesign unless explicitly requested;
5. update this file only when a design-system change is intentionally accepted.

The coding agent must not invent a new visual language for each feature.
