# Logos

Logos of the databases and AI providers SayQL works with, shown only to identify them. Using a
logo doesn't mean its owner endorses SayQL. Each logo is a trademark of its owner.

- Every `<id>.svg` here is a single-path, 24×24 icon from [Simple Icons](https://simpleicons.org)
  16.36.0 (CC0 1.0), copied unchanged. `logos.json` lists each one's name, brand color, original
  source and the brand's usage guidelines; `dark: true` marks a color too dark to read on the dark
  theme, where the logo is drawn in the text color instead.
- `scripts/third_party_notices.py` lists them in THIRD-PARTY-NOTICES.txt from `logos.json`.
- Brands without a clean source (Microsoft SQL Server and Azure SQL among them) get a neutral icon
  in `src/components/ui/Logo.tsx` instead of their logo.

To add one: copy `icons/<slug>.svg` from the `simple-icons` npm package, add its entry from the
package's `data/simple-icons.json` (`hex`, `source`, `guidelines`) to `logos.json`, and check the
brand's guidelines allow showing it to name the product.
