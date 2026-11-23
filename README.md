# Arcanum 2027

A reimplementation of *Arcanum: Of Steamworks and Magick Obscura* (2001) in Rust, built on the
[Bevy](https://bevyengine.org) game engine.

## Status

**This project is still under development.** It is not playable yet. What exists today:

- `arcanum_vfs`: reads the game's `.dat` archives and layers them with an override folder.
- `arcanum_formats`: decodes `.mes` message files and `.art` sprites.
- `arcanum_client`: loads assets through Bevy and shows the main menu at modern resolutions.

Planned for the finished game:

- 4K art: the original art upscaled on the player's own machine, from their own copy.
- A 3D mode with new art made from scratch.
- Resampled audio.
- Updated animations.

## Game data

This repository contains no original game assets, and the finished game will not ship upscaled
versions of them: the client upscales the original art on the player's machine. **Players will
still need a valid copy of Arcanum on [Steam](https://store.steampowered.com/)** to play. The
client reads the game data from that installation.

Point the client at the installed game folder (the one with `arcanum1.dat`):

```sh
ARCANUM_DIR="/path/to/Arcanum" cargo run -p arcanum_client
```

Without `ARCANUM_DIR`, the client looks in `assets/Arcanum`. `ARCANUM_OVERRIDE` adds a folder of
loose files that take priority over the archives.

## Development

```sh
cargo test --workspace
```

Most tests use synthetic data and run without the game. Tests that need the real game data read
`ARCANUM_DIR` and skip when it is not available.

## License

Released under the [MIT License](LICENSE).

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

The MIT License covers this project's source code only. It does not cover the original game's
assets, anything derived from them (such as art upscaled on a player's machine or resampled audio),
or the new art packs, which carry their own licences. *Arcanum: Of Steamworks and
Magick Obscura* and its assets are the property of their respective owners. This project is not
affiliated with or endorsed by them.
