# UI fonts

Preloaded by `src/routes/__root.tsx`; `@font-face` rules live in `src/styles.css`.
Keep preload URLs identical to the CSS source URLs to avoid duplicate downloads.

- Allo (body/UI): from the `allo-font` npm package (OFL-1.1), imported via
  `allo-font/allo.css`. Variable `wght` 100–900 plus obliques.
  Source: https://allo.joshpuckett.me
- `StackSansNotch-SemiBold-latin-v5.woff2`: Google's published Latin WOFF2
  subset at weight 600, used for the `socialguard` wordmark (`.font-socialguard`).
  `font-display: optional`. License: `StackSansNotch-OFL.txt`.
  Source: https://fonts.gstatic.com/s/stacksansnotch/v5/TwMY-JcVXlQd3ooGEx9EbUzgioTr5BY5lEpidqlSR8c8vi2eEmY.woff2
  License source: https://github.com/google/fonts/blob/main/ofl/stacksansnotch/OFL.txt
