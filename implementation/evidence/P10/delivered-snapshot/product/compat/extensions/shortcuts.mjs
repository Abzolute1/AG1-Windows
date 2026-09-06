// Conflict behavior ported from the pinned AG1 extension runner. Resolved
// keybindings come from the native caller; this module has no keyboard access.
const reserved = new Set([
	"app.interrupt",
	"app.clear",
	"app.exit",
	"app.suspend",
	"app.thinking.cycle",
	"app.model.cycleForward",
	"app.model.cycleBackward",
	"app.model.select",
	"app.tools.expand",
	"app.thinking.toggle",
	"app.editor.external",
	"app.message.followUp",
	"app.plan.review",
	"app.plan.nextStep",
	"tui.input.submit",
	"tui.select.confirm",
	"tui.select.cancel",
	"tui.input.copy",
	"tui.editor.deleteToLineEnd",
]);
export function resolveShortcuts(extensions, keybindings) {
	const builtin = new Map(),
		shortcuts = new Map(),
		diagnostics = [];
	const entries = Object.entries(keybindings ?? {});
	if (entries.length > 1024) throw new Error("Keybinding limit");
	for (const [keybinding, keys] of entries) {
		if (keys === undefined) continue;
		const list = Array.isArray(keys) ? keys : [keys];
		if (list.length > 64) throw new Error("Keybinding alternative limit");
		for (const key of list) {
			if (typeof key !== "string" || key.length > 256)
				throw new Error("Invalid keybinding");
			builtin.set(key.toLowerCase(), {
				keybinding,
				restrictOverride: reserved.has(keybinding),
			});
		}
	}
	const note = (message, path) => {
		if (diagnostics.length >= 4096)
			throw new Error("Shortcut diagnostic limit");
		diagnostics.push({ type: "warning", message, path });
	};
	for (const extension of extensions)
		for (const [key, shortcut] of extension.shortcuts) {
			const normalized = key.toLowerCase(),
				binding = builtin.get(normalized);
			if (binding?.restrictOverride) {
				note(
					`Extension shortcut '${key}' from ${extension.path} conflicts with built-in shortcut. Skipping.`,
					extension.path,
				);
				continue;
			}
			if (binding)
				note(
					`Extension shortcut conflict: '${key}' is built-in shortcut for ${binding.keybinding} and ${extension.path}. Using ${extension.path}.`,
					extension.path,
				);
			const previous = shortcuts.get(normalized);
			if (previous)
				note(
					`Extension shortcut conflict: '${key}' registered by both ${previous.extension.path} and ${extension.path}. Using ${extension.path}.`,
					extension.path,
				);
			shortcuts.set(normalized, { extension, shortcut });
		}
	return { shortcuts, diagnostics };
}
