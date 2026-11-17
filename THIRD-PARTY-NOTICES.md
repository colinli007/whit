# Third-party notices

Whit is licensed under the MIT license (see `LICENSE`). It includes or
adapts the following works, which keep their own licenses.

## gpui-component

`vendor/gpui-component` is a copy of [gpui-component](https://github.com/longbridge/gpui-kit)
0.9.1, Copyright 2024 - 2026 Longbridge, licensed under the Apache License 2.0
(`vendor/gpui-component/LICENSE-APACHE`). Whit modifies it: each changed file
says so at its top, and `patches/` holds every change against the release.

## Visual Studio Code

Parts of Whit follow the source of [Visual Studio Code](https://github.com/microsoft/vscode)
1.117, under the MIT license below:

- the indentation and bracket pair guides (`vendor/gpui-component/src/diff/guides.rs`),
  ported from `guidesTextModelPart.ts` and `indentGuides.ts`;
- the diff overview ruler (`vendor/gpui-component/src/diff/overview.rs`), from
  `overviewRulerFeature.ts`, `overviewZoneManager.ts` and `scrollbarState.ts`;
- the Monokai Dimmed colors (`assets/themes/monokai-dimmed.json`) and the default
  colors in `src/theme.rs`, from `extensions/theme-monokai-dimmed` and VS Code's
  color registry.

```
MIT License

Copyright (c) 2015 - present Microsoft Corporation

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

## tree-sitter-javascript

`vendor/gpui-component/src/highlighter/languages/javascript/highlights-jsx.scm`
is adapted from `queries/highlights-jsx.scm` of
[tree-sitter-javascript](https://github.com/tree-sitter/tree-sitter-javascript) 0.23.1,
Copyright (c) 2014 Max Brunsfeld, under the MIT license: the same terms as the
license above, with that copyright line.

## vscode-icons

The file icons in `assets/file-icons/icons` come from
[vscode-icons](https://github.com/vscode-icons/vscode-icons), Copyright (c) 2016
Roberto Huertas. The icons are licensed under
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) and are used
unmodified. The extension's code, from whose icon theme `assets/file-icons/theme.json`
is derived, is under the MIT license (`assets/file-icons/LICENSE-vscode-icons.txt`).
Branded icons remain under their owners' copyright and trademarks.
