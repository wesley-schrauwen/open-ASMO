# TODO

Things to pick up later. Larger plans live in `plans/`; project state and findings in `NOTES.md`.

## Art

- **Loading screen pocket watch:** the hand steps visibly because `LoadIndicator.ART` is 30
  pre-drawn frames (12° apart), and at one turn per 8 s each frame shows for ~0.27 s. The owner
  will make a replacement (GIF or animated SVG) with a smoothly rotating hand; the loading screen
  (`loading.rs`, `LoadingIndicator`) then needs to play it instead of the `.art` frames.
