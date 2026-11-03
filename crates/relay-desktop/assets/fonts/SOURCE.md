# Atkinson Hyperlegible Next source

Recorded: 2026-11-03.

RelayPost's native desktop interface embeds the unmodified Regular (400) and
Medium (500) static TrueType fonts from the official
[Atkinson Hyperlegible Next repository](https://github.com/googlefonts/atkinson-hyperlegible-next).
The Linux CLI does not embed these fonts. The existing egui emoji and monospace
fonts retain their separate upstream notices.

All three upstream files are pinned to commit
[`7925f50f649b3813257faf2f4c0b381011f434f1`](https://github.com/googlefonts/atkinson-hyperlegible-next/tree/7925f50f649b3813257faf2f4c0b381011f434f1).
The files were obtained through GitHub's repository Contents API at this commit.
No conversion, subsetting, renaming or font build was performed.

| Local file | Upstream path | Size (bytes) | SHA-256 |
| --- | --- | ---: | --- |
| `AtkinsonHyperlegibleNext-Regular.ttf` | [`fonts/ttf/AtkinsonHyperlegibleNext-Regular.ttf`](https://raw.githubusercontent.com/googlefonts/atkinson-hyperlegible-next/7925f50f649b3813257faf2f4c0b381011f434f1/fonts/ttf/AtkinsonHyperlegibleNext-Regular.ttf) | 65068 | `88ed5c31a71584c7772963b02d04bef1eb7e3d2e9c8b9cb204339b1f82cf432c` |
| `AtkinsonHyperlegibleNext-Medium.ttf` | [`fonts/ttf/AtkinsonHyperlegibleNext-Medium.ttf`](https://raw.githubusercontent.com/googlefonts/atkinson-hyperlegible-next/7925f50f649b3813257faf2f4c0b381011f434f1/fonts/ttf/AtkinsonHyperlegibleNext-Medium.ttf) | 67080 | `dd50b08b3c560846097d23baaaf6a97ffa20dd077115d23c59df68083b9ea05e` |
| `OFL.txt` | [`OFL.txt`](https://raw.githubusercontent.com/googlefonts/atkinson-hyperlegible-next/7925f50f649b3813257faf2f4c0b381011f434f1/OFL.txt) | 4431 | `aca6a428580965d2297d1b718042dd427c2a9443ece3b0d02d758e161e0c4030` |

The fonts are licensed under the **SIL Open Font License, Version 1.1**
(`OFL-1.1`). The complete, unedited upstream license and copyright notice are in
the adjacent `OFL.txt`:

> Copyright 2020-2024 The Atkinson Hyperlegible Next Project Authors (https://github.com/googlefonts/atkinson-hyperlegible-next)

The fonts retain their OFL terms when embedded; RelayPost's own code retains its
MIT license. This attribution does not imply endorsement by the font authors or
Braille Institute.

`third-party/desktop-fonts.json` records the same pinned inputs. macOS packaging
verifies their hashes, preserves `OFL.txt` and this source record in the package
and app resources, and records their provenance in `BUNDLED_FONTS.json` and
`THIRD_PARTY_NOTICES.md`. CI also verifies that both complete TTF byte sequences
are present in the prebuilt desktop executable. The font sources and notices
are not added to the Linux CLI's distribution inventory.
