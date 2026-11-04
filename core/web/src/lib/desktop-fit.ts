/**
 * What the desktop view tells the manager on its VNC socket, in a text frame beside the binary
 * frames of RFB: whether **Resize to window** fits the desktop's screen to this window. Once no
 * viewer fits it any more, because the last one turned the toggle off or its socket closed, the
 * manager gives the desktop the resolution its settings name again. The manager's
 * `resolution/fit.rs` takes these messages alone, and both sides hold them to
 * `tests/desktop-fit.json`.
 */
export function fitMessage(on: boolean) {
  return JSON.stringify({ fit: on })
}

/** Tell the manager whether this window fits the screen, while the socket is open. */
export function tellFit(socket: { readyState: number; send(data: string): void }, on: boolean) {
  if (socket.readyState === WebSocket.OPEN) socket.send(fitMessage(on))
}
