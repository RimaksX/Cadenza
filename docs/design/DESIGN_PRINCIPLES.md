# Design principles

What Cadenza's interface is, and why each value is the value it is.

**This document is descriptive.** The design was not specified in advance and
then implemented; it was decided screen by screen while the windows were built,
and the owner has confirmed that what is on screen is what they want
([MASTER_ISSUES 53](../MASTER_ISSUES.md)). So this file writes down a design
that already exists, in order to keep the next screen consistent with the last
one.

**`crates/ui/slint/theme/tokens.slint` is normative.** Every number below lives
there, and if the two ever disagree the file the compiler reads is right. What a
file of numbers cannot carry is why a number is that number, and that is what
this document is for.

The visual language came from a mockup — `Cadenza.html`, a single page at the
repository root that supplied the look and nothing else: never structure,
naming or behaviour. It was removed before release, because a mockup outlives
its use the moment the thing it described exists, and this document plus the
token file now say everything it said. It is in the history if anybody needs to
see where a colour came from.

## The one rule underneath everything: even lengths

Every length in the token file is even, and every halving in markup is rounded
as `round(x / 1px / 2) * 1px`. Half of what an interface does is centre one
thing inside another, and a centred child of an odd-sized parent lands on half a
pixel: its border blurs, and if it is text its letters rasterise to different
heights. Even sizes make the arithmetic come out whole without anybody having to
remember to round it.

The type scale is the deliberate exception. Those sizes were measured against
these faces rather than chosen for their parity, and a glyph's line box is
fractional whatever number is asked for.

`python scripts/audit_ui.py` enforces this and four related rules on every line
of markup. Each check exists because the defect it names shipped once and had to
be found by eye.

**And it is why the interface scales in its lengths rather than in its
renderer.** Every length in the markup is written as `Theme.px(n)`: the design
value multiplied by the size the listener chose and snapped back onto an even
whole pixel. A renderer told to draw at 125 per cent instead puts a one-pixel
hairline on a pixel and a quarter and every stem between two columns, which is
what a magnifying glass looks like ([MASTER_ISSUES 65](../MASTER_ISSUES.md)).
Type goes through `Theme.type-size(n)` and lands on a whole pixel rather than an
even one, because a glyph has no border to blur and nothing centred inside it.

## Type

Three faces, three jobs.

| Face | Job | What it is |
|---|---|---|
| Cadenza Serif | names things — a page, a playlist, a track, the wordmark | Cormorant Garamond |
| Cadenza Sans | explains them — prose, and anything read as a sentence | Manrope |
| Cadenza Mono | anything read as a number rather than as a word — captions, column heads, quantities, times | JetBrains Mono |

The families are bundled with the binary, so they need no network: a player that
is offline by requirement cannot have a typeface that arrives over one.

**They answer to names nobody else has, and that is what makes them identical on
every machine.** Slint asks for a family by name, and the system's own copies are
in the same database and were loaded first — so a listener with Cormorant
Garamond installed would have seen their copy, of whatever version, and a
developer would have seen their JetBrains Mono. Nothing but Cadenza can answer to
"Cadenza Serif". The renaming touches the name table and nothing else
(`scripts/rename_font.py`); all three are OFL 1.1 with no reserved name, and each
folder says what its file was before.

One weight each, in one file each, so no size or emphasis has to be asked for by
number — and a weight that has to be requested is a weight the system copy could
answer better.

**The sans is the Medium cut, and that is a rendering decision rather than a
taste one.** At 14px Manrope Regular's stems are about 1.05 pixels and nothing
in this stack hints them, so a stem that lands between two pixel columns is
drawn as two faint ones instead of one solid: in a capital M the right-hand stem
lost half its density and the letter read as though it had been cut off. The
Medium stems cross the same grid intact. The serif needs no such help — measured
at title size its stems come out solid — so it ships as Regular.

**Four sizes, and nothing between them.** Display 40, title 24, body 14, label
12 — the serif sizes a step above where they started, because Cormorant Garamond
has a much smaller lower case than the face it replaced and the old numbers read
a size down ([MASTER_ISSUES 59](../MASTER_ISSUES.md)). Display names a page or an object; title names a thing inside one; body is
prose; label is the mono voice. Four rather than seven because a size that
exists gets used, and a seventh size is a seventh thing for the eye to sort.
Emphasis comes from the face and the ink instead.

**Mono capitals are set wide** — two pixels of letter-spacing at label size.
Upper-case mono crowds itself, and a caption is read as a shape before it is
read as words. Digits get a hair of the same treatment, half a pixel, so a
column of times reads as a column rather than as a word.

**Every line of type states its own box, and the box is never smaller than the
face needs.** The serif asks for about 1.21 times its size and the sans for 1.37.
A row that leaves less does not clip the line — Slint drops it entirely, which is
a defect that looks like missing data rather than like a layout mistake. The
audit holds every boxed line to 1.35 times its type size, which is the tighter of
the two with a little to spare.

## Colour

Four surfaces (`bg-0`..`bg-3`), five inks (`ink-0`, `ink-1`, `ink-2`,
`ink-mute`, `ink-disabled`), two rules (`line`, `line-soft`) — and one colour.

`danger` is the only hue in either palette. It marks a control that takes
something away: remove from the library, delete a playlist, delete a preset,
replace the copy you already have, tell a station you want less of this. It
appears only under the pointer, so the interface stays grey until the moment
something is about to be destroyed. Warm rather than a signal red, so it belongs
to this palette instead of arriving from a system dialog.

Contrast is measured rather than judged; the table is in
[UI_CONTRACT.md](../UI_CONTRACT.md). Every ink that carries text clears 4.5:1 on
the surfaces it appears on. `ink-disabled` is deliberately below that minimum: a
disabled control cannot look unavailable while it is as legible as the controls
that work.

**Both themes share one geometry.** Light is not a second design; it is the same
drawing with the surfaces and inks exchanged. The light values are chosen rather
than mirrored — a hairline that reads as quiet on near-black disappears on
near-white, and a red that warns on dark reads as pink on light.

## Space

One series — 4, 8, 12, 16, 24, 32 — so nothing lands between two steps.

A page is inset 56 pixels from the window edge rather than run to it. The
separators are then the width of the content and not of the screen, which is
what stops a long list reading as a spreadsheet.

**A part states its own width; nothing is stretched into place.** A figure on
the listening page is 176, the album column is 180, a button is as wide as its
longest word needs. What a page divides into is the exception: the settings page
is one column of the page's own width, because two columns of stated widths left
the second one holding a single control and half the window empty
([MASTER_ISSUES 73](../MASTER_ISSUES.md)). Pages fill the window because their
parts add up to something, not because something expands. A `min-width` under a
page that would not otherwise fill the window is a crutch: it makes the page a
fixed size instead of giving its contents a size, which is the same mistake one
level up ([MASTER_ISSUES 52](../MASTER_ISSUES.md)).

## Shape

Corners are 4 pixels, or a full circle where the thing is a dot, a dial or a
handle. There is nothing in between and no second radius.

**Objects are drawn with borders and surfaces, never with shadows.** There is
not one drop shadow in the interface. Depth is which of the four surfaces a
thing stands on.

**One button shape.** A rectangle with a hairline border, mono capitals inside
it, and a width stated in pixels. A destructive one is the same shape in the
danger ink; a disabled one is the same shape in `ink-disabled`. There is no
second kind of button anywhere, so a control is recognised as a control before
it is read.

## Motion

120 milliseconds, ease-out, on hover and selection. Long enough not to snap,
short enough that nobody waits for it.

**Hover belongs to the whole object, not to the part under the pointer.** A row
lights as one thing, and the controls inside it appear because the row is
hovered rather than because each of them was found. An interface where the
pointer discovers hidden targets one at a time is a search task.

**Transitions, not running animations.** Something that moves costs frames while
it moves and nothing afterwards. The one thing that runs continuously — the
visualiser — stops when the window is not on screen.

## Endings and continuations

A **line** is how this interface ends something: the rule after a section head,
the separator under a track row, the edge of a card.

Therefore a line cannot also mean "there is more below". A list that continues
past its bottom edge **fades** into the ground it stands on
(`components/ScrollFade.slint`). Nothing in a page ends by dissolving, so
something that dissolves is being cut off — which is unambiguous in a way a
second hairline was not.

The fade is used only on secondary lists: moods, presets, the settings column,
the standings, the decisions. The library and the queue *are* the page rather
than a part of it, and a page fading at the bottom would say the music runs out
there.

## The shape of a page

Every screen opens the same way: a mono label saying what this is, a display
name in serif, and a mono line of quantities under it — "THE LAST THIRTY DAYS /
Listening / 41 listens over 30 days", "BUILT FROM YOUR OWN LIBRARY / Radio",
"HELD BACK RATHER THAN IMPORTED / Decisions".

**An empty state says why it is empty and what fills it**, in that order, and as
a sentence rather than as a label: "History is off for this profile, so nothing
you play is written down. Turn it on in Settings and this page fills up as you
listen." A screen that says only "No data" makes the listener guess whether
something is broken.

**A place exists only while it has something to say.** Decisions appear in the
sidebar only while a file is waiting for one. A permanently empty room is a room
nobody looks in.

## What is deliberately not here

No shadows. No second accent colour. No icon where a word is clearer — the
sidebar carries both because a destination is found by shape at a glance, but a
button says what it does in words. No gradient except the one that means "this
continues". No animation that runs while nothing is happening. No third-party
widget set: every control is built from Slint's unstyled primitives, so it wears
this design rather than the toolkit's.
