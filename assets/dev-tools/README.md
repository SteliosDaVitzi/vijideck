Language and ecosystem logos are vendored from Devicon v2.17.0:
https://github.com/devicons/devicon/tree/v2.17.0/icons

Each `<name>.svg` corresponds to `icons/<name>/<name>-original.svg`.
The upstream MIT license is included in LICENSE.devicon. Logos remain the
property of their respective owners. The original SVGs are unmodified.

build.svg, packages.svg and tools.svg are VijiDeck category illustrations.

These files are compiled into the application. No network requests, executable
inspection, or user-specific paths are needed to display developer group icons.
Add new group mappings in src/environment_icons.rs and extend its rendering test
when adding a new group to the environment inventory.
