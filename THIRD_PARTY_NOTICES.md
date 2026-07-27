# Third-party notices

HOI4 Map Editor is based on ScottyThePilot's HOI4 Province Editor. The
original project copyright and MIT licence are preserved in `LICENSE`.

HOI4 Map Editor is an unofficial community tool. It is not affiliated with or
endorsed by Paradox Interactive. No Hearts of Iron IV or mod assets are
distributed.

## Bundled font

Inconsolata is embedded in the executable.

Copyright 2006 The Inconsolata Project Authors

This Font Software is licensed under the SIL Open Font License, Version 1.1:

Permission is hereby granted, free of charge, to any person obtaining a copy
of the Font Software, to use, study, copy, merge, embed, modify, redistribute,
and sell modified and unmodified copies, subject to these conditions:

1. Neither the Font Software nor any individual component may be sold by
   itself.
2. Original or Modified Versions may be bundled, redistributed, or sold with
   software if each copy contains the copyright notice and this licence.
3. A Modified Version may not use a Reserved Font Name without permission.
4. Copyright-holder and author names may not promote a Modified Version,
   except as acknowledgement or with permission.
5. The Font Software must remain under this licence. Documents created with
   the Font Software are not subject to that requirement.

THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
OR IMPLIED, INCLUDING MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE COPYRIGHT HOLDER BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY ARISING FROM THE FONT SOFTWARE.

The complete SIL Open Font License 1.1 is retained in the source repository at
`assets/Inconsolata-OFL.txt`.

## Bundled icons

The inherited UI spritesheet contains icons attributed by the original project
to Tabler Icons and css.gg.

Tabler Icons: Copyright (c) 2020-2026 Paweł Kuna.

Legacy css.gg icons: Copyright (c) 2019 css.gg.

Both icon sets are provided under the MIT License:

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

The exact upstream revisions used to assemble the inherited spritesheet were
not recorded. This provenance gap remains a manual release gate; the notice
above preserves the licences and attributions present in the original project.

## Rust dependencies

The Rust crates used by the application are recorded exactly in `Cargo.lock`
and remain subject to their own licences. Direct Git dependencies are pinned
by the lockfile. The automated audit confirmed no local path dependencies, but
it did not independently prove every transitive crate's licence metadata.
That limitation must be reviewed before a public release.
