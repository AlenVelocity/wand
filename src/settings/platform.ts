// Which system the settings window is running on, for the words and keys
// that differ. WebKit on macOS says "Macintosh" in its user agent.
export const isMac = /Macintosh|Mac OS X/.test(navigator.userAgent);
