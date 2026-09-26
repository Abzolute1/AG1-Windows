import { expect, test } from "vitest";
import { normalizePreferences, defaults, applyPreferences } from "./preferences";
test("persisted preferences are bounded, typed and cannot inject styles", () => {
  const p = normalizePreferences({ uiSize: -500, chatSize: 900, statusSize: "17", motion: "anything", background: "url(https://example.test)", accent: "#aabbcc", pointers: "false", chatFont: "__proto__" });
  expect(p.uiSize).toBe(13); expect(p.chatSize).toBe(28); expect(p.statusSize).toBe(17); expect(p.motion).toBe("system"); expect(p.background).toBe(""); expect(p.accent).toBe("#aabbcc"); expect(p.pointers).toBe(true); expect(p.chatFont).toBe("system");
});
test("each text surface uses its own preference and resetting removes color overrides", () => {
  applyPreferences({ ...defaults, uiSize: 16, chatSize: 23, codeSize: 12, statusSize: 18, accent: "#ffffff" });
  const s = document.documentElement.style; expect(s.getPropertyValue("--chat-font-size")).toBe("23px"); expect(s.getPropertyValue("--status-font-size")).toBe("18px"); expect(s.getPropertyValue("--code-font-size")).toBe("12px"); expect(s.getPropertyValue("--primary-foreground")).toBe("#151518"); applyPreferences(defaults); expect(s.getPropertyValue("--primary")).toBe("var(--theme-primary)");
});
