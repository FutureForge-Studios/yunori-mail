# Console UI kit (`src/ui`)

Hand-written building blocks for the console (`/console`) and the mailbox portal
(`/account`). They **compose** the generated layers and never edit them:

- brand tokens: `src/design/yunori/tokens.css` (generated from yunori.email)
- client primitives: `src/design/vendor/**` (generated from the client repo)
- refinements: `src/styles/app.css` (plain CSS: `color-scheme`, skeleton shimmer,
  `.display-number`, `.brand-wash`, layout variables) and the theme extensions in
  `tailwind.config.ts` (`text-2xs`, `shadow-card`, `animate-rise-in`, …).

## Direction

Warm paper canvas (light) / night ink (dark), cards one step lighter with a
hairline and a whisper shadow, Fraunces for page titles and big numbers, Geist for
everything else, torii vermilion used sparingly (active nav marker, primary
connect/sign-in actions, tab underline). Status colour always ships with an icon
or a word. Focus is a 2px ring with offset everywhere. Body text is 13–14px; the
smallest text is 11px (`text-2xs`) and only for labels.

## Modules

| File | Exports | Use for |
|---|---|---|
| `page.tsx` | `Page`, `PageContainer`, `PageHeader`, `Stack`, `usePageTitle` | Every screen. `width="wide" \| "default" \| "narrow"`; header takes `eyebrow`, `title`, `blurb`, `meta` (badges), `actions`. |
| `card.tsx` | `Card`, `CardHeader`, `CardBody`, `CardFooter`, `SectionHeader`, `SectionLabel`, `KV`, `KVList` | Surfaces. `Card padded={false}` + `CardHeader icon title description actions divider` for sectioned cards. `KV` rows go inside `KVList` (a `<dl>`). |
| `button.tsx` | `Button` (vendored variants + `loading`), `IconButton` (`label` required) | Actions. |
| `form.tsx` | `Input`, `Textarea`, `Select` (native, styled), `Label`, `Field`, `FieldHint`, `FieldError`, `Switch`, `SwitchRow`, `Checkbox`, `CheckboxRow`, `RadioRow`, `RadioCard`, `AddressInput`, `selectClass`, `fieldSurface` | Forms. `RadioCard` is the big choice card (sending mode, invite method); `compact` for tiles. |
| `segmented.tsx` | `Segmented` (radiogroup), `Tabs` (underlined tablist), `SearchInput` | Filters, view switches, in-card tabs, table search. |
| `badge.tsx` | `StatusBadge`, `Badge`, `Chip`, `Kbd`, `StatusDot`, `Avatar` | State and labels. |
| `tone.ts` | `Tone` and class maps (`TONE_TEXT`, `TONE_SOFT`, `TONE_DOT`, `TONE_FILL`, `TONE_STROKE`) | `ok \| warn \| bad \| info \| muted`. |
| `data-viz.tsx` | `StatCard`, `Meter`, `RingMeter`, `BarList`, `SegmentBar`, `usageTone` | Numbers and small charts; every mark has its value in text. |
| `data-table.tsx` | `DataTable`, `Pager`, `TableToolbar`, `Column` | Tables. Below `md` each row renders as a card (`mobile: "title" \| "aside" \| "field" \| "hidden"` per column), so nothing scrolls sideways on phones. |
| `feedback.tsx` | `EmptyState`, `LoadError`, `Callout`, `FormError`, `Skeleton`, `InlineLoading` | Empty / loading / error states and inline banners. |
| `overlay.tsx` | `Dialog*` (phone-safe sizing, serif title), `Sheet` (slide-over) | Dialogs and drawers. |
| `steps.tsx` | `Steps`, `Step` (`done \| current \| waiting \| failed \| pending \| skipped`) | Any process: DMARC ladder today, the install checklist next. |

Toasts live in `src/lib/toast.tsx` (`toast.success/error/info`), the shell in
`src/console/shell.tsx`, the sign-in frame in `src/components/auth-frame.tsx`
(`AuthFrame`: brand panel + form column; pass `aside` to replace the panel copy).

`src/components/kit.tsx` re-exports the common pieces under their old names, so
older imports (`Page`, `Card`, `DataTable`, `FilterPills`, …) keep working.

## Restyling the onboarding screens

`setup.tsx`, `setup-complete.tsx` and `install-checklist.tsx` were left as they
were. To bring them over: `AuthFrame` (already restyled) for setup; `Card
padded={false}` + `CardHeader` for each block; `Steps`/`Step` for the checklist
(map `StepStatus` `running → current`, `waiting → waiting`, `failed → failed`,
`done → done`, `skipped → skipped`, `pending → pending`); `Callout` for the
"what to do" boxes; `Button`, `Input`, `Label`, `Field` from `@/ui/*` instead of
the vendored ones (the vendored `Label` is an uppercase eyebrow).
