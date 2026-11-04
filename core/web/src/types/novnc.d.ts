// The part of noVNC's RFB API the desktop view uses; see node_modules/@novnc/novnc/docs/API.md.
declare module "@novnc/novnc" {
  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      urlOrChannel: string | WebSocket,
      options?: { shared?: boolean; wsProtocols?: string[] }
    )
    scaleViewport: boolean
    resizeSession: boolean
    // noVNC 1.7 dropped it from API.md with the constructor option it deprecated, but the property
    // is still there; check core/rfb.js before raising noVNC.
    showDotCursor: boolean
    focusOnClick: boolean
    viewOnly: boolean
    disconnect(): void
    focus(): void
    clipboardPasteFrom(text: string): void
  }
}
