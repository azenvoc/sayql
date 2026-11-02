# Explainer video

"Let agents into your database. Not your data." Claude Code asks a question through SayQL and
gets counts, not rows; a private column is refused; the app's Agent activity shows it and
blocks an app; people ask in the app; an audit learns the database from its code and docs, and
a note whose column was renamed is flagged with a fix; and a team server gives an agent its
own key, a scope and an audit trail.

Records the real app UI (the browser preview with `src/dev/mockBackend.ts`) and a throwaway
SayQL team server's admin page, and turns them into `media/explainer.mp4`,
`media/explainer-light.mp4`, `media/explainer-15s.mp4` and `media/explainer-poster.png`. Needs
Google Chrome, Node 22, Python 3 with numpy, ffmpeg, and a `sayql-server` binary (0.3.0 or
later, for the agent gateway).

```bash
pnpm exec vite --port 1431 --strictPort          # keep running
SAYQL_SERVER_BIN=/path/to/sayql-server scripts/video/team-server.sh   # keep running; prints an admin link
SAYQL_ADMIN_LINK='http://127.0.0.1:8092/admin#code=…' \
  SAYQL_TEAM_DB="$PWD/scripts/video/out/team-server/shop.db" \
  node scripts/video/capture.mjs dark            # ~110 s, real time
# restart team-server.sh (a new link, empty data), then the same for light
node scripts/video/render.mjs dark full          # ~3 min each
node scripts/video/render.mjs dark short
node scripts/video/render.mjs dark poster
node scripts/video/render.mjs light full
node scripts/video/render.mjs dark stills 12,40  # single frames at those times, to check an edit
```

- `capture.mjs` holds the storyboard: what to type and click, plus the camera targets
  (element rects) and captions. It opens the preview with `?video`, which gives the mock
  answers for each scene and an Agent activity log that matches the terminal scenes. For the
  knowledge scene it gives the demo store two knowledge sources (a code folder and a docs
  site) and a scripted audit; when the audit ends, the note "products.price is in cents" turns
  doubtful (the column is `price_cents` in `?video`) with SayQL's fix waiting. Then it
  opens the team server's admin page, adds the Shop connection (with `customers.email` never
  sent), a person and their question off camera, and on camera adds the agent "Support bot",
  scopes it to two tables and tenant 42's orders, and opens the audit log after the agent has
  run three queries over MCP (one refused).
- `render.mjs` holds the edit (scene order, the 15-second cut, the site on the end card) and
  draws each frame with `compositor.html`: the window on the brand background, the eased
  zoom, the cursor, the captions, the title and end cards, and the two terminal scenes. Those
  are drawn, not recorded: Claude Code calling SayQL's `run_query` over MCP, with the answers
  in the shape `src-tauri/src/mcp/shape.rs` gives (a summary on a counts connection, and its
  never-send refusal). Keep them in step with that file.
- Captures go to `scripts/video/out/` (git-ignored, ~500 MB each).

## Sound

- **Music:** the videos use the track in `music.json` ("Corporate Tech" by MomotMusic, from
  Pixabay, under the Pixabay Content License). The licence doesn't allow sharing the file on
  its own, so it isn't in the repo: download it from the `download` URL in `music.json` to its
  `file` path before rendering. It's cut to length with a fade. Without it, `music.py`'s
  original synthesized track is used. `MUSIC_FILE=path/to/track.mp3` picks another track
  and `MUSIC=0` renders silent; add `audio` to the render command to redo only the sound.
- **Voiceover:** the lines and their cue points are in `voiceover.json`; each render writes
  the script with start times and how long each line can run to `media/<name>-cuts.md`.
  The full cuts use the lines in `media/voiceover/<id>.wav`, with the music ducked under the
  voice; the 15-second cut is music only. The current takes are Kokoro's `am_michael` (see
  below). Read every changed line with that same voice, or the video switches voices:
  `voiceover.py am_michael <dir>` reads them all; copy the changed lines into
  `media/voiceover/`, and check each still fits its room in the cuts file. A line's `speak`
  spells it for macOS voices ("Say Q L"). Then run
  `node scripts/video/render.mjs dark full audio` (only the soundtrack is redone). Other voices:
  - recordings: one file per line, in `media/voiceover/<id>.wav`;
  - Kokoro, an open-source voice model (Apache-2.0): set it up and read the lines as in
    `voiceover.py`'s header, then render with `VO_DIR=<its out dir>`. Good voices to try:
    `af_heart`, `am_michael`, `bf_emma`;
  - a macOS voice: `VOICE="Ava (Premium)" node scripts/video/render.mjs dark full audio`.
    Premium voices are a free download in System Settings → Accessibility → Spoken Content
    → System voice → Manage Voices; the default ones sound robotic.

When the UI changes, re-run the capture and render. Element lookups go by text and ARIA
labels, so a renamed button needs the same rename in `capture.mjs`.
