import "@testing-library/jest-dom";

// jsdom does not implement window.matchMedia; polyfill it for theme tests.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: (_type: string, _listener: () => void) => {
      // no-op; tests that need to change the result should spy/replace this
    },
    removeEventListener: () => {},
    dispatchEvent: () => true,
  }),
});
