import { ImageResponse } from "next/og"
const size = { width: 1200, height: 630 }
export function GET() {
  return new ImageResponse(
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        width: "100%",
        height: "100%",
        padding: 80,
        background: "#20231e",
        color: "#fafaf8",
        fontFamily: "sans-serif",
      }}
    >
      <div style={{ fontSize: 36 }}>VoicePaste</div>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          fontSize: 76,
          lineHeight: 1.1,
        }}
      >
        <span>Think freely.</span>
        <span>Speak naturally.</span>
      </div>
      <div style={{ fontSize: 26 }}>Free. Lightweight. Open source.</div>
    </div>,
    size
  )
}
