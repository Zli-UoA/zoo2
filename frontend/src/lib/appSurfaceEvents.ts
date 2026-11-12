export const APP_SURFACES_CHANGED = "zoo:app-surfaces-changed";

/** Wake private UI polling after a command or action can publish a modal/response. */
export function refreshAppSurfaces() {
  window.dispatchEvent(new Event(APP_SURFACES_CHANGED));
}
