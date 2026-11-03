/** Prepended to every key; empty except in a development preview (see `setStorageNamespace`). */
let prefix = "";

/** localStorage access that never throws (private mode, quota, disabled storage). */
export const storage = {
  get<T>(key: string, fallback: T): T {
    try {
      const raw = localStorage.getItem(prefix + key);
      return raw === null ? fallback : (JSON.parse(raw) as T);
    } catch {
      return fallback;
    }
  },
  set(key: string, value: unknown): void {
    try {
      localStorage.setItem(prefix + key, JSON.stringify(value));
    } catch {
      // Preferences are a convenience; losing one is harmless.
    }
  },
};

/**
 * Development only (the mock backend's `?firstrun`): keeps the preview's remembered tabs and
 * preferences apart from the usual ones, starting empty when `fresh`, so a first run can be
 * previewed again and again without touching what the normal preview remembers.
 */
export function setStorageNamespace(namespace: string, fresh = false): void {
  prefix = namespace;
  if (!fresh || !namespace) return;
  try {
    for (const key of Object.keys(localStorage))
      if (key.startsWith(namespace)) localStorage.removeItem(key);
  } catch {
    // No storage: nothing to clear.
  }
}
