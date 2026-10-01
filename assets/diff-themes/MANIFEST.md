# Diff themes

Syntax-highlighting themes for diff-reckoner, as TextMate `.tmTheme` plists that
`syntect::highlighting::ThemeSet::load_from_reader` parses. Every file has an opaque `#RRGGBB`
`background` and `foreground` and at least 15 scope rules, and opens with an XML comment (just
after the `<?xml?>` declaration) giving its source, licence and copyright.

Converted files also carry, where the source theme defines them, these extra global settings
(syntect ignores them): `diffInserted` / `diffDeleted` (line background, from
`diffEditor.insertedLineBackground` / `removedLineBackground`, falling back to the
`...TextBackground` key) and `diffInsertedText` / `diffDeletedText` (from
`diffEditor.insertedTextBackground` / `removedTextBackground`).

Order is by popularity: the three required defaults first, then VS Code Marketplace installs of
the extension that ships the theme (queried 2026-10-01; dark and light variants share one count).
VS Code built-ins (Monokai, Solarized, Quiet Light) have no install count and are placed by
judgement.

## Dark

| # | File | Display name | Type | Upstream | Licence | Copyright | Installs | Produced |
|---|---|---|---|---|---|---|---|---|
| 1 | `xcode-dark.tmTheme` | Xcode Default (Dark) | dark | Apple Xcode built-in "Default (Dark)" (colour values) | MIT (diff-reckoner) | Colour values: Apple | 0.14M (required default) | Hand-authored: Apple's sRGB palette mapped onto TextMate scopes |
| 2 | `vscode-dark.tmTheme` | Dark Modern (VS Code) | dark | <https://github.com/microsoft/vscode/blob/main/extensions/theme-defaults/themes/dark_modern.json> | MIT | Copyright (c) 2015 - present Microsoft Corporation | n/a (required default) | Converted from VS Code JSON (include chain dark_modern -> dark_plus -> dark_vs resolved; selection and diff colours are the VS Code colour-registry defaults the theme inherits (src/vs/platform/theme/common/colors/editorColors.ts)) |
| 3 | `github-dark.tmTheme` | GitHub Dark Default | dark | <https://github.com/primer/github-vscode-theme> | MIT | Copyright (c) 2020 Primer | 20.2M | Converted from VS Code JSON (GitHub.github-vscode-theme v6.3.5 (generated themes/dark-default.json)) |
| 4 | `one-dark-pro.tmTheme` | One Dark Pro | dark | <https://github.com/Binaryify/OneDark-Pro> | MIT | Copyright (c) 2013-2022 Binaryify | 12.8M | Converted from VS Code JSON (zhuangtongfa.Material-theme v3.20.2 (generated themes/OneDark-Pro.json)) |
| 5 | `dracula.tmTheme` | Dracula | dark | <https://github.com/dracula/visual-studio-code> | MIT | Copyright (c) 2016 Dracula Theme | 11.0M | Converted from VS Code JSON (dracula-theme.theme-dracula v2.25.1 (generated theme/dracula.json)) |
| 6 | `monokai.tmTheme` | Monokai (VS Code) | dark | <https://github.com/microsoft/vscode/blob/main/extensions/theme-monokai/themes/monokai-color-theme.json> | MIT | Copyright (c) 2015 - present Microsoft Corporation | built-in | Converted from VS Code JSON (VS Code built-in) |
| 7 | `ayu-dark.tmTheme` | Ayu Dark | dark | <https://github.com/ayu-theme/vscode-ayu> | MIT | Copyright (c) 2016 Ike Kurghinyan | 4.2M | Converted from VS Code JSON (teabyii.ayu v1.4.0 (generated ayu-dark.json)) |
| 8 | `winter-is-coming-dark.tmTheme` | Winter is Coming (Dark Blue) | dark | <https://github.com/johnpapa/vscode-winteriscoming> | MIT | Copyright (c) 2015-2017 JohnPapa.net, LLC | 3.7M | Converted from VS Code JSON (johnpapa.winteriscoming v1.5.0) |
| 9 | `night-owl.tmTheme` | Night Owl | dark | <https://github.com/sdras/night-owl-vscode-theme> | MIT | Copyright (c) 2018 Sarah Drasner | 3.6M | Converted from VS Code JSON (sdras.night-owl v2.1.1) |
| 10 | `tokyo-night.tmTheme` | Tokyo Night | dark | <https://github.com/enkia/tokyo-night-vscode-theme> | MIT | Copyright (c) 2018-present Enkia | 3.0M | Converted from VS Code JSON (enkia.tokyo-night v1.1.2) |
| 11 | `palenight.tmTheme` | Palenight | dark | <https://github.com/whizkydee/vscode-palenight-theme> | MIT | Copyright (c) 2017-present Olaolu Olawuyi | 2.6M | Converted from VS Code JSON (whizkydee.material-palenight-theme v2.0.5) |
| 12 | `synthwave-84.tmTheme` | SynthWave '84 | dark | <https://github.com/robb0wen/synthwave-vscode> | MIT | Copyright (c) 2019 Robb Owen | 2.6M | Converted from VS Code JSON (RobbOwen.synthwave-vscode v0.1.20 (base colours only; the neon glow is CSS injection, not part of the theme)) |
| 13 | `shades-of-purple.tmTheme` | Shades of Purple | dark | <https://github.com/ahmadawais/shades-of-purple-vscode> | MIT | Copyright (c) 2015-present Ahmad Awais | 2.3M | Converted from VS Code JSON (ahmadawais.shades-of-purple v7.3.6) |
| 14 | `cobalt2.tmTheme` | Cobalt2 | dark | <https://github.com/wesbos/cobalt2-vscode> | MIT | Copyright (c) 2018 Wes Bos, Roberto Achar | 1.9M | Converted from VS Code JSON (wesbos.theme-cobalt2 v2.5.0) |
| 15 | `solarized-dark.tmTheme` | Solarized Dark (VS Code) | dark | <https://github.com/microsoft/vscode/blob/main/extensions/theme-solarized-dark/themes/solarized-dark-color-theme.json> | MIT | Copyright (c) 2015 - present Microsoft Corporation | built-in | Converted from VS Code JSON (VS Code built-in) |
| 16 | `catppuccin-mocha.tmTheme` | Catppuccin Mocha | dark | <https://github.com/catppuccin/vscode> | MIT | Copyright (c) 2021 Catppuccin | 1.4M | Converted from VS Code JSON (Catppuccin.catppuccin-vsc v3.19.0 (generated themes/mocha.json)) |
| 17 | `noctis.tmTheme` | Noctis | dark | <https://github.com/liviuschera/noctis> | MIT | Copyright (c) 2018 Liviu Schera | 1.4M | Converted from VS Code JSON (liviuschera.noctis v10.43.3) |
| 18 | `nord.tmTheme` | Nord | dark | <https://github.com/nordtheme/visual-studio-code> | MIT | Copyright (C) 2017-present Arctic Ice Studio; Copyright (C) 2017-present Sven Greb | 1.3M | Converted from VS Code JSON (arcticicestudio.nord-visual-studio-code v0.19.0) |
| 19 | `gruvbox-dark.tmTheme` | Gruvbox Dark Medium | dark | <https://github.com/jdinhify/vscode-theme-gruvbox> | MIT | Copyright (c) 2017 JD | 1.1M | Converted from VS Code JSON (jdinhlife.gruvbox v1.29.1) |
| 20 | `rose-pine.tmTheme` | Rosé Pine | dark | <https://github.com/rose-pine/vscode> | MIT | Copyright (c) 2021 Rosé Pine | 0.34M | Converted from VS Code JSON (mvllow.rose-pine v2.15.2 (generated)) |

## Light

| # | File | Display name | Type | Upstream | Licence | Copyright | Installs | Produced |
|---|---|---|---|---|---|---|---|---|
| 1 | `xcode-light.tmTheme` | Xcode Default (Light) | light | Apple Xcode built-in "Default (Light)" (colour values) | MIT (diff-reckoner) | Colour values: Apple | 0.14M (required default) | Hand-authored: Apple's sRGB palette mapped onto TextMate scopes |
| 2 | `vscode-light.tmTheme` | Light Modern (VS Code) | light | <https://github.com/microsoft/vscode/blob/main/extensions/theme-defaults/themes/light_modern.json> | MIT | Copyright (c) 2015 - present Microsoft Corporation | n/a (required default) | Converted from VS Code JSON (include chain light_modern -> light_plus -> light_vs resolved; selection and diff colours are the VS Code colour-registry defaults the theme inherits (src/vs/platform/theme/common/colors/editorColors.ts)) |
| 3 | `github-light.tmTheme` | GitHub Light Default | light | <https://github.com/primer/github-vscode-theme> | MIT | Copyright (c) 2020 Primer | 20.2M | Converted from VS Code JSON (GitHub.github-vscode-theme v6.3.5 (generated themes/light-default.json)) |
| 4 | `ayu-light.tmTheme` | Ayu Light | light | <https://github.com/ayu-theme/vscode-ayu> | MIT | Copyright (c) 2016 Ike Kurghinyan | 4.2M | Converted from VS Code JSON (teabyii.ayu v1.4.0 (generated ayu-light.json)) |
| 5 | `winter-is-coming-light.tmTheme` | Winter is Coming (Light) | light | <https://github.com/johnpapa/vscode-winteriscoming> | MIT | Copyright (c) 2015-2017 JohnPapa.net, LLC | 3.7M | Converted from VS Code JSON (johnpapa.winteriscoming v1.5.0) |
| 6 | `light-owl.tmTheme` | Night Owl Light | light | <https://github.com/sdras/night-owl-vscode-theme> | MIT | Copyright (c) 2018 Sarah Drasner | 3.6M | Converted from VS Code JSON (sdras.night-owl v2.1.1) |
| 7 | `tokyo-night-light.tmTheme` | Tokyo Night Light | light | <https://github.com/enkia/tokyo-night-vscode-theme> | MIT | Copyright (c) 2018-present Enkia | 3.0M | Converted from VS Code JSON (enkia.tokyo-night v1.1.2) |
| 8 | `one-light.tmTheme` | Atom One Light | light | <https://github.com/akamud/vscode-theme-onelight> | MIT | Copyright (c) 2015 Mahmoud Ali | 1.5M | Converted from VS Code JSON (akamud.vscode-theme-onelight v2.3.0) |
| 9 | `catppuccin-latte.tmTheme` | Catppuccin Latte | light | <https://github.com/catppuccin/vscode> | MIT | Copyright (c) 2021 Catppuccin | 1.4M | Converted from VS Code JSON (Catppuccin.catppuccin-vsc v3.19.0 (generated themes/latte.json)) |
| 10 | `noctis-lux.tmTheme` | Noctis Lux | light | <https://github.com/liviuschera/noctis> | MIT | Copyright (c) 2018 Liviu Schera | 1.4M | Converted from VS Code JSON (liviuschera.noctis v10.43.3) |
| 11 | `gruvbox-light.tmTheme` | Gruvbox Light Medium | light | <https://github.com/jdinhify/vscode-theme-gruvbox> | MIT | Copyright (c) 2017 JD | 1.1M | Converted from VS Code JSON (jdinhlife.gruvbox v1.29.1) |
| 12 | `solarized-light.tmTheme` | Solarized Light (VS Code) | light | <https://github.com/microsoft/vscode/blob/main/extensions/theme-solarized-light/themes/solarized-light-color-theme.json> | MIT | Copyright (c) 2015 - present Microsoft Corporation | built-in | Converted from VS Code JSON (VS Code built-in) |
| 13 | `quiet-light.tmTheme` | Quiet Light (VS Code) | light | <https://github.com/microsoft/vscode/blob/main/extensions/theme-quietlight/themes/quietlight-color-theme.json> | MIT | Copyright (c) 2015 - present Microsoft Corporation | built-in | Converted from VS Code JSON (VS Code built-in) |
| 14 | `min-light.tmTheme` | Min Light | light | <https://github.com/miguelsolorio/min-theme> | MIT | Copyright (c) 2018-2021 Miguel Solorio | 0.63M | Converted from VS Code JSON (miguelsolorio.min-theme v1.5.0) |
| 15 | `horizon-bright.tmTheme` | Horizon Bright | light | <https://github.com/jolaleye/horizon-theme-vscode> | MIT | Copyright (c) 2018 Jonathan Olaleye | 0.45M | Converted from VS Code JSON (jolaleye.horizon-theme-vscode v2.0.2) |
| 16 | `rose-pine-dawn.tmTheme` | Rosé Pine Dawn | light | <https://github.com/rose-pine/vscode> | MIT | Copyright (c) 2021 Rosé Pine | 0.34M | Converted from VS Code JSON (mvllow.rose-pine v2.15.2 (generated)) |
| 17 | `gruvbox-material-light.tmTheme` | Gruvbox Material Light | light | <https://github.com/sainnhe/gruvbox-material-vscode> | MIT | Copyright (c) 2020 sainnhe | 0.34M | Converted from VS Code JSON (sainnhe.gruvbox-material v6.5.2 (default-configuration theme file)) |
| 18 | `tomorrow.tmTheme` | Tomorrow | light | <https://github.com/chriskempson/tomorrow-theme/blob/master/textmate/Tomorrow.tmTheme> | MIT | Copyright (C) 2011 Chris Kempson | 0.22M | Vendored tmTheme (upstream TextMate file; also the source of the TomorrowKit extension whose install count is shown) |
| 19 | `everforest-light.tmTheme` | Everforest Light | light | <https://github.com/sainnhe/everforest-vscode> | MIT | Copyright (c) 2020 sainnhe | 0.13M | Converted from VS Code JSON (sainnhe.everforest v0.3.0 (default-configuration theme file)) |
| 20 | `flexoki-light.tmTheme` | Flexoki Light | light | <https://github.com/kepano/flexoki/blob/main/vscode/Flexoki-Light-color-theme.json> | MIT | Copyright (c) 2023 Steph Ango | n/a (not on Marketplace; 3.7k GitHub stars for the palette repo) | Converted from VS Code JSON (kepano/flexoki main branch) |

## Notes

- **Xcode**: `xcode-dark` / `xcode-light` are hand-authored for diff-reckoner from the colours of
  Apple's built-in Default (Dark) / (Light) themes, checked against an Xcode screenshot, with keywords
  bold as Xcode draws them. Xcode colours some roles from semantic analysis (project vs system
  symbols, declarations) that regex grammars cannot see; those map to the nearest scope, so a
  grammar that does not mark a role leaves it plain. Their diff colours are Xcode's comparison view
  (dark sampled from a screenshot: grey and orange for deletions, slate and blue for insertions;
  light is the same hues lightened, with no reference yet).
- **VS Code defaults**: `dark_modern` / `light_modern` set neither a selection colour nor diff
  colours, so VS Code falls back to its colour registry. Those registry defaults are written into
  `vscode-dark` / `vscode-light` explicitly so they match what VS Code shows.
- **Partial diff keys**: `one-dark-pro` and `one-light` define only the inserted colour upstream, so
  they have `diffInserted` / `diffInsertedText` and no `diffDeleted*`.
- **No diff keys**: `solarized-dark`, `solarized-light`, `quiet-light`,
  `light-owl`, `tomorrow`.
- `tokyo-night-light` declares `"type": "dark"` upstream, but its colours are light.
- Converted rules keep only `bold` / `italic` / `underline` font styles (syntect rejects others,
  such as `strikethrough`). Rules with an empty scope list (four palette entries in Flexoki) are
  dropped.
- Left out: Panda (no licence), Bluloco (LGPL-3.0), Material Theme / Monokai Pro (not
  permissively licensed).
- Converted themes were taken from the extension package (`.vsix`) published on the VS Code
  Marketplace at the version shown, because several upstream repos generate their theme JSON at
  build time. VS Code built-ins, Flexoki and Tomorrow were fetched from the GitHub repos linked
  above. The converter was a throwaway Rust tool outside the repo (it resolves `include` chains,
  normalises colours to `#RRGGBB` / `#RRGGBBAA`, and checks every scope selector with syntect's
  parser).
