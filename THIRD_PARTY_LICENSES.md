# Third-party licenses

## libcosmic (MPL-2.0)

`Window::popup_container_with_opacity` in `src/window.rs` is a derivative of
`cosmic::applet::Context::popup_container`, from `src/applet/mod.rs` in
[libcosmic](https://github.com/pop-os/libcosmic). libcosmic is licensed under
the Mozilla Public License 2.0, not the MIT license that covers the rest of this
crate, and that function is therefore subject to the MPL.

The applet calls the upstream helper directly in the default case; the
derivative exists only because upstream bakes its container style in with no
hook to override, so the `background_opacity` setting cannot reach the alpha any
other way. Its source is right here in this repository, as the MPL requires.

The full license text is at
<https://github.com/pop-os/libcosmic/blob/master/LICENSE>, and a copy ships with
the crate itself.

## Provider icons

The SVG files under `data/icons/providers/` are vendored from
[CodexBar](https://github.com/steipete/CodexBar), where they live as
`Sources/CodexBar/Resources/ProviderIcon-<slug>.svg`. They are embedded into
this applet's binary at compile time by `src/icons.rs` and are used
unmodified apart from the filename losing its `ProviderIcon-` prefix.

CodexBar is MIT licensed:

```
MIT License

Copyright (c) 2026 Peter Steinberger

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
