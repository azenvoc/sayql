/**
 * The SayQL logo, "shape, not data": a database whose schema (violet, what the AI sees) sits above
 * rows that stay out of sight (dotted blue). Same artwork as src-tauri/icons/app-icon.svg, cropped to
 * the tile; below 28px it draws the simplified mark used for 16 and 32px icons.
 *
 * The hex fills in this file are fixed on purpose: brand colors match the app icon and logo
 * everywhere, so they don't follow the light/dark theme tokens.
 */
export function BrandMark({ size = 18, className }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" className={className} aria-hidden="true">
      {size < 28 ? <SmallMark /> : <FullMark />}
    </svg>
  );
}

function Tile() {
  return <rect width="100" height="100" rx="24" fill="#1d2125" />;
}

function FullMark() {
  return (
    <>
      <Tile />
      <ellipse cx="50" cy="29" rx="27" ry="9.5" fill="#a78bfa" />
      <g fill="#4a8dff">
        <circle cx="23.00" cy="36.75" r="2.5" />
        <circle cx="77.00" cy="36.75" r="2.5" />
        <circle cx="23.00" cy="44.50" r="2.5" />
        <circle cx="77.00" cy="44.50" r="2.5" />
        <circle cx="23.00" cy="51.00" r="2.5" />
        <circle cx="77.00" cy="51.00" r="2.5" />
        <circle cx="23.00" cy="57.50" r="2.5" />
        <circle cx="77.00" cy="57.50" r="2.5" />
        <circle cx="23.00" cy="64.00" r="2.5" />
        <circle cx="77.00" cy="64.00" r="2.5" />
        <circle cx="23.00" cy="70.50" r="2.5" />
        <circle cx="77.00" cy="70.50" r="2.5" />
        <circle cx="35.00" cy="52.40" r="2.5" />
        <circle cx="42.47" cy="53.62" r="2.5" />
        <circle cx="50.21" cy="54.00" r="2.5" />
        <circle cx="57.74" cy="53.60" r="2.5" />
        <circle cx="65.18" cy="52.36" r="2.5" />
        <circle cx="35.00" cy="65.40" r="2.5" />
        <circle cx="42.47" cy="66.62" r="2.5" />
        <circle cx="50.21" cy="67.00" r="2.5" />
        <circle cx="57.74" cy="66.60" r="2.5" />
        <circle cx="65.18" cy="65.36" r="2.5" />
        <circle cx="27.91" cy="75.96" r="2.5" />
        <circle cx="35.00" cy="78.40" r="2.5" />
        <circle cx="42.47" cy="79.62" r="2.5" />
        <circle cx="50.21" cy="80.00" r="2.5" />
        <circle cx="57.74" cy="79.60" r="2.5" />
        <circle cx="65.18" cy="78.36" r="2.5" />
        <circle cx="72.21" cy="75.90" r="2.5" />
      </g>
    </>
  );
}

function SmallMark() {
  return (
    <>
      <Tile />
      <ellipse cx="50" cy="33" rx="30" ry="12" fill="#a78bfa" />
      <path
        d="M20 61a30 12 0 0 0 60 0"
        fill="none"
        stroke="#4a8dff"
        strokeWidth="9"
        strokeLinecap="round"
        strokeDasharray="0.01 15"
      />
    </>
  );
}

/** The SayQL name next to the mark: the name in the text color (JetBrains Mono, as outlines). */
export function Wordmark({ height = 18, className }: { height?: number; className?: string }) {
  return (
    <svg
      height={height}
      width={Math.round((height * 410) / 148)}
      viewBox="0 0 410 148"
      className={className}
      role="img"
      aria-label="SayQL"
    >
      <g transform="translate(0 24)">
        <SmallMark />
      </g>
      <g transform="translate(117 0)" fill="currentColor">
        <path d="M30.400000000000002 111.5Q19.8 111.5 13.5 105.9Q7.2 100.3 7.2 90.5H16.2Q16.2 96.6 20.1 100.05Q24.0 103.5 30.400000000000002 103.5Q36.6 103.5 40.3 99.9Q44.0 96.3 44.0 90.5Q44.0 86.1 41.650000000000006 82.85Q39.300000000000004 79.6 34.9 78.4L23.8 75.3Q16.900000000000002 73.3 12.950000000000001 68.15Q9.0 63.0 9.0 56.0Q9.0 50.199999999999996 11.600000000000001 45.8Q14.200000000000001 41.39999999999999 18.900000000000002 38.949999999999996Q23.6 36.5 29.700000000000003 36.5Q35.9 36.5 40.650000000000006 38.949999999999996Q45.400000000000006 41.39999999999999 48.10000000000001 45.74999999999999Q50.800000000000004 50.099999999999994 50.800000000000004 55.8H41.800000000000004Q41.800000000000004 50.8 38.400000000000006 47.65Q35.0 44.5 29.700000000000003 44.5Q24.400000000000002 44.5 21.1 47.65Q17.8 50.8 17.8 55.8Q17.8 59.8 19.950000000000003 62.65Q22.1 65.5 26.0 66.6L37.4 69.8Q44.7 71.8 48.75 77.35Q52.800000000000004 82.9 52.800000000000004 90.5Q52.800000000000004 100.1 46.7 105.8Q40.6 111.5 30.400000000000002 111.5ZM82.2 111.5Q73.7 111.5 68.7 106.75Q63.7 102.0 63.7 94.3Q63.7 89.2 66.0 85.4Q68.3 81.6 72.4 79.44999999999999Q76.5 77.3 81.8 77.3H98.80000000000001V73.0Q98.80000000000001 62.3 87.1 62.3Q81.9 62.3 78.7 64.19999999999999Q75.5 66.1 75.3 69.5H66.3Q66.8 63.0 72.35 58.75Q77.9 54.5 87.1 54.5Q97.1 54.5 102.45 59.3Q107.80000000000001 64.1 107.80000000000001 72.69999999999999V110.5H98.9V100.5H98.7Q97.9 105.6 93.60000000000001 108.55Q89.30000000000001 111.5 82.2 111.5ZM84.4 103.9Q91.0 103.9 94.9 100.7Q98.80000000000001 97.5 98.80000000000001 92.0V84.3H82.8Q78.4 84.3 75.65 86.94999999999999Q72.9 89.6 72.9 94.0Q72.9 98.6 75.95 101.25Q79.0 103.9 84.4 103.9ZM131.8 128.5 139.9 107.1 119.4 55.5H129.1L142.1 89.5Q142.9 91.5 143.60000000000002 93.7Q144.3 95.9 144.6 97.5Q144.9 95.9 145.60000000000002 93.7Q146.3 91.5 147.0 89.5L159.1 55.5H168.6L141.2 128.5Z" />
        <path
          transform="translate(-24 0)"
          d="M240.9 128.5 230.6 111.5Q230.3 111.5 230.0 111.5Q223.0 111.5 217.8 108.8Q212.6 106.1 209.7 101.19999999999999Q206.8 96.3 206.8 89.7V58.3Q206.8 51.699999999999996 209.65 46.8Q212.5 41.89999999999999 217.75 39.199999999999996Q223.0 36.5 230.0 36.5Q237.0 36.5 242.2 39.199999999999996Q247.4 41.89999999999999 250.3 46.8Q253.2 51.699999999999996 253.2 58.3V89.7Q253.2 96.3 250.35 101.25Q247.5 106.2 242.3 108.8L254.4 128.5ZM230.0 100.5Q235.2 100.5 237.95 97.65Q240.7 94.8 240.7 89.7V58.3Q240.7 53.199999999999996 237.85 50.3Q235.0 47.4 230.0 47.4Q225.0 47.4 222.15 50.3Q219.3 53.199999999999996 219.3 58.3V89.7Q219.3 94.8 222.05 97.65Q224.8 100.5 230.0 100.5ZM269.1 110.5V37.5H281.6V99.2H312.6V110.5Z"
        />
      </g>
    </svg>
  );
}
