/** Narrow gate so commands can refuse addPanel during fromJSON without importing lifecycle. */

let fromJsonPending = false;

export function setLayoutFromJsonPending(pending: boolean): void {
  fromJsonPending = pending;
}

export function isLayoutRestoring(): boolean {
  return fromJsonPending;
}
