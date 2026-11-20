# Heuristic review of the native window, November 2026

The egui window against Nielsen's heuristics, from `app/shots/` (19 Nov, 01:15 batch), `app/src` and the mockups. Stale shots (`offset-found`, `edit`, `drawer-picker`, `picker-branch`, `recipe-menu`, `recipe-delete`) aren't cited. Line numbers are approximate; `app/` is being edited.

Severity 1–4. **U** uncontroversial, **D** needs a decision.

## 1. Visibility of system status

**1.1 Rip says "verified" for tracks that aren't.** Severity 3, U.
- **What:** `tray.rs:296` reads "{ok} verified against AccurateRip", and the *Verified* tab (`tray.rs:~1619`) counts the same thing. Both count every `ItemState::Ok`, which includes *Kept, unverified*, *Combined* (warn tone) and *Two reads agree*. With AccurateRip off, the header still says "verified against AccurateRip".
- **Why:** the one number an accuracy-first user trusts overstates.
- **Fix:**
  - Count only items whose `rip_verdict` is *Verified*.
  - The sentence names the rest ("15 verified, 1 kept unverified"); with AccurateRip off, "17 read, not checked".

**Done.** Only *Verified* counts as verified (the tab and the sentence); the sentence names the rest ("15 verified, 1 kept unverified"), or "17 read, not checked".

**1.2 Failures to save or load only reach stderr.** Severity 3, U.
- **What:**
  - `model.rs` `save()`/`save_settings()` and `open_folder` (`main.rs:42`) only `eprintln!`.
  - With `recipes.json` unreadable (`main.rs:54`), `store.rs` `save_recipes` returns `Ok` without writing: a whole session of edits silently dropped.
  - An unreadable `settings.json` is overwritten on the next save, losing the drives' offsets.
- **Why:** the window looks fine while nothing is being kept.
- **Fix:**
  - A red header chip that stays until fixed: "Not saving: recipes.json can't be read", with *Open folder*. Folder-open errors use it too.
  - Guard `save_settings` like `save_recipes`.

**Done.** A red header chip ("Not saving: recipes.json can't be read", or the error) with *Open folder*, until it's fixed; folder-open errors use it. An unreadable `settings.json` is never overwritten.

**1.3 Idle tiles show amber "waiting" states.** Severity 2, U.
- **What:** in `complex-fit` and `loop` the recipe is *Ready*, yet NAS, Backup and *Tell me* show an amber clock and "Waits for library".
- **Why:** amber means "needs you"; the design's not-started tile is grey *After …*.
- **Fix:** before a run, downstream tiles show grey `idle_status` ("After library").

**Done**, in the app: before a run nothing waits in amber; tiles read "After …".

**1.4 Half the recipe is off-screen, with no sign of it.** Severity 2, D.
- **What:** in `idle`, `running` and `ask`, everything after Rip is past the right edge at 100%, while the canvas's lower half is empty.
- **Fix:** fit on first show, capped at 100%.
- **Decision:** the design fixes tiles at 100%; the alternative is a fade and arrow at an edge that hides tiles.

## 2. Match between system and the real world

**2.1 Every bad track is called "Damaged".** Severity 3, U.
- **What:** `tray.rs:~1315` (`rip_verdict`) shows the verdict *Damaged* for "no match", and `drawer-rip` shows "Damaged / No match in AccurateRip".
- **Why:** `DESIGN-UX.md`'s *Unverified* section says no match after a fast read isn't evidence of damage, and the glossary's verdicts are *verified, consistent, unverified, doesn't match*. The mockup shows "no match".
- **Fix:**
  - *Doesn't match* for no match; *Damaged* only for "Reads disagree: …"; *Couldn't be read* for an empty note.
  - *Two reads agree* becomes *Consistent*, the glossary word.

**Done.** *Doesn't match* for no match, *Damaged* only when reads disagree, *Couldn't be read* for nothing back; *Two reads agree* is *Consistent*.

## 3. User control and freedom

**3.1 Closing the window during a run or hold asks nothing.** Severity 3, U.
- **What:** there is no `close_requested` handling (`lib.rs` `Window::ui`). Closing mid-rip kills the run, and held discs, which live only in memory, vanish from the sidebar.
- **Fix:** when any run is busy or holding, send `ViewportCommand::CancelClose`. The header then asks "Quit? CDs is ripping" with *Keep* and a red *Quit*, the same pattern as recipe delete.

**Done.** Closing with a run going or a disc held is cancelled and asked in the header: "Quit? CDs is ripping", *Keep*, red *Quit*. The window's own close button goes the same way.

**3.2 Leaving a recipe with unsaved edits is a trap.** Severity 2, U.
- **What:** picking another recipe mid-draft shows a small "Save changes?" (`app.rs:714`). The only answers are Save, or Cancel (discard and leave). There's no way to stay, and Esc doesn't clear `leave_to`.
- **Fix:** while `leave_to` is set, the header shows "Save changes to *CDs*?" with *Stay*, *Discard* and *Save* (primary). Esc means Stay.

**Done.** "Save changes to CDs?" with *Stay*, *Discard* and *Save*; Esc stays.

**3.3 Esc is too blunt.** Severity 2, U.
- **What:** `app.rs:318` closes the drawer even during a Remove confirm (it should mean *Keep*), and with a name field focused, which commits on `lost_focus`, so Esc saves the typing.
- **Fix:** Esc undoes the innermost thing: a focused field reverts, then a confirm becomes *Keep*, then an expanded drawer restores, then the drawer closes.

**Done.** A field being typed in goes back to its value; then a confirm keeps (Quit, leaving, Delete, Remove); then an expanded drawer comes back; then it closes.

**3.4 No way back to a default.** Severity 2, U.
- **What:** nothing returns a *yours* setting to *default*, or a typed read offset to AccurateRip's value. The design's *Reset to built-in* doesn't exist.
- **Fix:** a quiet *Reset* after a *yours* tag (`Setting::Default`; for the offset, re-run the lookup), and *Reset to built-in* in the seeded recipes' edit header.

**Done.** A quiet *Reset* after *yours* (an offset is looked up again); *Reset to built-in* in a seeded recipe's edit header, as one undoable edit.

**3.5 Settings changed while editing ignore Cancel.** Severity 2, D.
- **What:** `model.rs:~592` ("Only structure is drafted") and `set_hold` save at once, even in a draft, so *Cancel* reverts a tile name but not a format change.
- **Decision:** deliberate per the comment. Draft everything while editing, or say "Settings save as you go" by Save/Cancel.

**3.6 Stop has no confirm.** Severity 2, D.
- **What:** `app.rs:724`, next to Edit; a stopped rip can't resume.
- **Fix:** Delete's inline confirm ("Stop? 6 of 17 read", *Keep*, *Stop*).
- **Decision:** a click more on a deliberate action.

## 4. Consistency and standards

**4.1 One state, several words.** Severity 2, U.
- **What:**
  - A bad track reads "1 to retry" in the sidebar and header, "1 needs you" in the drawer pill (`tray.rs` `state_pill`), and "Damaged" in the row.
  - Header chips are lower case ("needs you", "running", "ready", `app.rs:631-647`), while drawer pills are capitalised.
  - The tile says "offset 103" but the field says "+103".
- **Fix:**
  - Bad items read "1 to retry" in the sidebar, the header and the pill. The tab stays *Needs you*.
  - Capitalise the chips like the pills.
  - Format the offset with its sign everywhere.

**Done.** "1 to retry" in the pill too; header chips capitalised; the tile reads "offset +103".

**4.2 Step names lower-cased in "After …".** Severity 1, U.
- **What:** `crates/engine/src/snapshot.rs:174` lower-cases the name, giving "After audio cd", "After flac" and "After musicbrainz".
- **Fix:** drop `to_lowercase()`.

**Done**, in the app: the engine's lower-casing is overridden from the tiles' own names ("After Audio CD"), as the engine wasn't to be edited for this.

**4.3 Scrolling zooms.** Severity 2, D.
- **What:** in `canvas.rs:84-91`, any scroll zooms and horizontal scroll is ignored. A two-finger swipe on a trackpad zooms instead of panning.
- **Fix:** scroll pans both ways, and Ctrl+scroll or a pinch zooms.
- **Decision:** deliberate per the comment; node editors differ.

## 5. Error prevention

**5.1 Bad input is thrown away.** Severity 2, U.
- **What:** in `tray.rs:~884`, a value that doesn't parse discards the typed text and flashes "Read offset?" in the header for 3 s. That is the same words as the not-set prompt.
- **Fix:**
  - Keep the text, outlined red, with the reason in the provenance slot ("a whole number, like +103"). Commit nothing until it parses.

**Done.** The text stays, outlined red, with the reason under it ("A whole number, like +103"); nothing is saved until it reads.

## 6. Recognition rather than recall

**6.1 Choose doesn't show how each candidate fits the disc.** Severity 3; ticks U, Search D.
- **What:**
  - In `ask`, both releases carry the same "✓ disc ID ✓ 17 titles", and the track list has no per-track check against the disc's lengths.
  - Titles are cut short ("The Big Three Killed …") in two narrow columns.
  - The mockup has "Tracks, against the disc" with a ✓ per track, and a *Search* action.
- **Fix:**
  - A ✓ or an amber length difference on each track row, against the TOC; one column when narrow.
- **Decision (Search):** needs a text lookup in the engine; scope.

**Done** (the ticks): each track has a ✓ against the disc, or an amber note (not on the disc, not in this release, a length difference when the release gives lengths); one column when narrow. Search still needs a decision.

**6.2 Start's disabled reason is in engine words.** Severity 2, U.
- **What:** `cant_start` (`model.rs:397`) gives the tooltip texts "unsaved", "1 to set", "doesn't fit", "drive busy" and "not available". "Doesn't fit" doesn't say which step.
- **Fix:**
  - Sentences: "Save or cancel your edits first", "Library needs a folder".
  - For "doesn't fit", mark the failing tile red, as Remove marks its tiles.

**Done.** Sentences ("Library needs folder", "Save or cancel your edits first", "The drive is busy with CDs"); a tile that doesn't fit is marked red.

## 8. Aesthetic and minimalist design

**8.1 "default" under every control.** Severity 1, D.
- **What:** `drawer-mp3-vbr` shows "default" eight times.
- **Fix:** show provenance only when it isn't the default (*yours, 19 Nov*; *AccurateRip, 528 agree*). No tag means default.
- **Decision:** the design says every value shows where it came from.

**8.2 Drawer headers repeat the tile.** Severity 1, U.
- **What:** pill "15 of 17" then "15 of 17 encoded, 61% overall" (`drawer-encoder-progress`); "MP3 as V0, joint stereo." above those settings (`drawer-mp3-vbr`).
- **Fix:** a working step's pill says only "Encoding"; an idle encoder has no sentence.

**Done.** A working encoder's pill says *Encoding*, with only "61% overall" beside it; an idle one has no sentence.

## 9. Help users recognise, diagnose and recover from errors

**9.1 Measure fails silently.** Severity 2, U.
- **What:** `Model::look_up` (`model.rs:174`) returns without doing anything when the drive is busy, yet the button stays enabled. A failed measure shows "not found" with no reason.
- **Fix:** disable it while busy, with the reason; on failure say why under the field ("No disc in the drive", "AccurateRip doesn't know this disc").

**Done.** Measure isn't offered while the drive is busy (its settings are locked then). A failed measure says "Not measured: it needs a disc in the drive that AccurateRip knows"; the engine doesn't say which, so one sentence covers both.

**9.2 Refusals vanish after 3 s, far from the cause.** Severity 2, U.
- **What:** `App::refuse` (`app.rs:260`) puts engine words ("last step", "running", "no drive yet") in a header chip that disappears after 3 s.
- **Fix:** show it by its cause (for a drawer action, the drawer's sentence slot) until the next click, as a sentence ("A recipe needs at least its drive").

**Done.** Refusals are sentences, shown in the open drawer's sentence (else the header) until the next click.

## Work order (uncontroversial only)

1. 1.1: count only real verifications.
2. 2.1: use the glossary's verdict words.
3. 1.2: show save and load failures; don't overwrite an unreadable settings file.
4. 3.1: confirm quitting during a run or hold.
5. 6.1: per-track ✓ in Choose, and one column when narrow.
6. 5.1, 9.2: keep bad input and say why, next to the cause.
7. 6.2: Start's reason as a sentence, and mark the tile that doesn't fit.
8. 3.2, 3.3: Stay/Discard/Save, and Esc that undoes the innermost thing.
9. 9.1: Measure's busy state and failure reasons.
10. 3.4: Reset on *yours*, and *Reset to built-in*.
11. 1.3: no amber on idle tiles.
12. 4.1, 4.2: one word per state, casing, the offset's sign.
13. 8.2: drop the repeated numbers from drawer headers.

Needs a decision: 1.4, 3.5, 3.6, 4.3, 6.1 (Search), 8.1.
