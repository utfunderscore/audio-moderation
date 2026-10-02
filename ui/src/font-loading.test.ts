/// <reference types="node" />

import { readFileSync } from "node:fs"
import { describe, expect, it } from "vitest"

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8")
const css = readFileSync(new URL("./index.css", import.meta.url), "utf8")
const fontFaces = [...css.matchAll(/@font-face\s*\{([^}]+)\}/g)].map(
  (match) => match[1]
)

describe("font loading", () => {
  it("self-hosts and preloads both fonts without allowing late swaps", () => {
    expect(fontFaces).toHaveLength(2)

    for (const face of fontFaces) {
      expect(face).toMatch(/font-display:\s*optional;/)
      expect(face).toMatch(/font-weight:\s*600;/)
      const source = face.match(/url\("(\/fonts\/[^"]+\.woff2)"\)/)?.[1]
      expect(source).toBeDefined()

      const preload = [...html.matchAll(/<link\b[^>]*>/g)].find((match) =>
        match[0].includes(`href="${source}"`)
      )?.[0]
      expect(preload).toBeDefined()
      expect(preload).toContain('rel="preload"')
      expect(preload).toContain('as="font"')
      expect(preload).toContain('type="font/woff2"')
      expect(preload).toMatch(/\bcrossorigin\b/)

      const binary = readFileSync(
        new URL(`../public${source}`, import.meta.url)
      )
      expect(binary.subarray(0, 4).toString("ascii")).toBe("wOF2")
    }
  })

  it("does not depend on Google Fonts", () => {
    expect(html).not.toMatch(/fonts\.(googleapis|gstatic)\.com/)
    expect(css).not.toMatch(/fonts\.(googleapis|gstatic)\.com/)
  })
})
