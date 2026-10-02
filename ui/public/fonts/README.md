# UI fonts

Both above-the-fold fonts are self-hosted and preloaded by `index.html`.
Their `@font-face` rules use `font-display: optional`: slow first visits keep
the fallback font for that page view instead of swapping text after it renders.
Keep preload URLs and CSS source URLs identical to avoid duplicate downloads.
Use a new filename when replacing a font binary.

- `IoskeleyMono-SemiBold.woff2`: existing full font, unchanged. Do not restrict
  its character coverage; it is used for filenames and transcripts.
- `StackSansNotch-SemiBold-latin-v5.woff2`: Google's published Latin WOFF2
  subset at weight 600, used only for the `socialguard` wordmark. No custom
  subsetting was performed. License: `StackSansNotch-OFL.txt`.
  Source: https://fonts.gstatic.com/s/stacksansnotch/v5/TwMY-JcVXlQd3ooGEx9EbUzgioTr5BY5lEpidqlSR8c8vi2eEmY.woff2
  License source: https://github.com/google/fonts/blob/main/ofl/stacksansnotch/OFL.txt
