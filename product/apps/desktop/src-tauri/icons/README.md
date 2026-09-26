# AG1 app icons

Artwork supplied by the app owner in `ag1_icon_sizes.zip`.

The 16, 24, 32, 48, 64, 128, 256, 512 and 1024 pixel PNGs and the multi-resolution Windows ICO are preserved from that archive. `icon.png` is the 1024 pixel master; `128x128@2x.png` is the supplied 256 pixel image.

The macOS ICNS, Windows tile icons, and Android/iOS assets were generated from the supplied 1024 pixel master using the repository's pinned Tauri CLI. The desktop configuration lists the PNG, ICO and ICNS assets explicitly. Linux installs all supplied sizes under the hicolor icon theme as `ag1`.

To regenerate platform derivatives, run the pinned Tauri `icon` command on the master, then restore the supplied PNG sizes and ICO. Original source archive SHA-256: `006f5912f4adc441e57950da9e60ed6f414930e61f3551d6b1d7692ccc2aefeb`.
