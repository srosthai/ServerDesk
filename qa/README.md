# ServerDesk QA evidence: 2026-11-26

Manual browser verification for issues #315, #316, and #317. Screenshots were
captured with Playwright against the Vite web preview at 1280x480, 1440x900,
and the phone-sized dialog viewport.

## Issue #317: Server profile dialog overflows short screens

- `issue-317-scroll/before-1280x480-add-server.png` shows the dialog clipped at
  the top and the Save server footer outside the viewport.
- `issue-317-scroll/before-1280x550-add-server.png` shows the same layout with
  the dialog starting above the viewport.
- `issue-317-scroll/after-1280x480-add-server.png` shows the constrained dialog
  with a visible Cancel, Test connection, and Save server footer. The form body
  scrolls inside the dialog.

Measured after the fix at 1280x480: dialog bounds `top: 16`, `bottom: 464`;
footer button bounds `top: 413`, `bottom: 449`; body overflow hidden.

## Issue #315: Inline editing in the Database Data tab

- `issue-315-inline-edit/before-inline-edit-1440x900.png` shows the Data tab
  before editing.
- `issue-315-inline-edit/inline-edit-active-1440x900.png` shows the in-cell
  editor open on the `name` column with the NULL toggle.
- `issue-315-inline-edit/inline-edit-after-enter-1440x900.png` shows the saved
  value, the committed-row banner, and the refreshed grid.

The Playwright run used a mocked Tauri row backend to exercise the real UI:
double-click opened the editor, Enter sent one field with its previous value as
the conflict guard, the update returned `affectedRows: 1`, and the grid
refreshed to `Ann Sok Updated`.

## Issue #316: Fullscreen Database workspace

- `issue-316-fullscreen/normal-1440x900.png` shows the normal shell.
- `issue-316-fullscreen/fullscreen-1440x900.png` shows the empty workspace
  using the full viewport.
- `issue-316-fullscreen/normal-data-1440x900.png` and
  `issue-316-fullscreen/fullscreen-data-1440x900.png` show a populated Data tab
  before and after entering fullscreen.

Measured in fullscreen at 1440x900: workspace `top: 0`, `left: 0`, `width:
1440`, `height: 900`, `position: fixed`; sidebar and topbar `display: none`;
body overflow hidden. Escape restored the static layout and sidebar.

## Automated checks

- `npm run check` passed: versions, typecheck, lint, Prettier, production build.
- `npm test -- --run` passed: 99 files, 689 tests.
- New tests cover inline commit, Escape cancel, invalid values, NULL writes, and
  model validation and conflict guards.
