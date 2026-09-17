---
title: The web UI
description: The same binary that answers the API serves the dashboards, on the same port - and every screen works on a phone.
---

The same binary that answers the API serves the web app, on the same port:
`http://127.0.0.1:8080` is the sign-in screen, and `/api/v1/...` is the API. A
self-hosted install is one file - there is no web server to configure, and no
way for the UI to be from a different build than the API it calls.

**My week** is the employee's own history: seven days, each drawn as a timeline
of gold stretches broken by the pauses in them, and any day opens to its pauses
and the tasks logged on it. **The team** is the manager's dashboard - a row per
person with their hours, bars that compare people with each other, and a status
that says what the server knows, under a band naming the people the server
thinks are worth a look; clicking a row opens that person's twelve-week chart
and their week, in the same component the personal page uses. **The month** is the same team as a
grid - a square per person per day, shaded by hours, with weekends marked from
the calendar and nothing recorded drawn as an empty square rather than as a
worked day of zero. Its five shades come from the design system rather than
from this product's own gold: mixed from the accent, the faintest step cleared
the empty square by 1.49:1, which made "barely worked" and "nobody reported"
the same square to anyone scanning the grid - and the empty square is the one
a manager is scanning for. The system's ramp clears it by 2.1:1. **My data** renders the
manifest from what the server actually enforces ([ADR 0011](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0011-the-privacy-manifest.md))
rather than describing it again in the page, so the two cannot disagree.

The team screens are hidden from an employee's navigation, but that is tidiness
rather than security: the endpoints behind them refuse an employee outright.

Where the installation's privacy level withheld something, the page says so in
that spot. A `coarse` day draws no timeline and states how many interruptions
there were and how long they came to, because an unbroken gold bar would be a
claim about the day that the server did not keep the evidence for.

The version in the header comes from `/health` - the server's, not the
bundle's. One product, one number.

## On a phone

Every screen works at 320px, and none of them is a cut-down version of itself:
a manager reading the dashboard on the way to work sees the same rows, the same
bars and the same signals.

What changes is the arrangement. The header's tabs move to a bar along the
bottom, where the thumb already is - a bar rather than a menu behind a button,
because three or four destinations fit across a phone as they are and a menu
would bring state, a focus trap and a way to be left open across a route
change. Both navigations are drawn from one list, so a screen cannot appear in
one and not the other.

Rows stack: on the personal week a day's name and its total share the top line
and the timeline takes the full width beneath them, because the desktop's three
columns would leave the bar about eighty pixels wide, which is a smudge rather
than a drawing of a day. The team's rows do the same. The month keeps its grid
and scrolls sideways inside its own box, with the names frozen at the left -
a month of squares is wider than a phone at any arrangement, and the alternative
is a shape that is no longer a month.

The trend chart drops the per-column week labels, which do not fit - twelve
columns is eighteen pixels each, and `Jun 22` needs twenty-eight - and names
the span under the chart instead. Every column still says its own week and
hours on touch.

The sign-in fields are 16px on a phone. Anything smaller and iOS Safari zooms
the page in on focus and does not zoom back out, which leaves someone signing
in at 130% with the password field off the side of the screen.
