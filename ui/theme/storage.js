// Small persistence wrappers for theme settings. Every access is guarded: private windows, blocked site data,
// previews and quota errors must never break the app — they only mean the choice is not remembered.
//
// - kv: JSON values in localStorage (falls back to memory)
// - blobs: files (fonts, logos) in IndexedDB (falls back to memory; small files also to localStorage data urls)

const memory = new Map();
const PREFIX = 'anamnesis.';

export const kv = {
  get(key, fallback = null) {
    try {
      const raw = globalThis.localStorage?.getItem(PREFIX + key);
      if (raw != null) return JSON.parse(raw);
    } catch {
      /* unavailable or corrupt: fall through */
    }
    return memory.has(key) ? memory.get(key) : fallback;
  },
  set(key, value) {
    memory.set(key, value);
    try {
      globalThis.localStorage?.setItem(PREFIX + key, JSON.stringify(value));
      return true;
    } catch {
      return false;
    }
  },
  remove(key) {
    memory.delete(key);
    try {
      globalThis.localStorage?.removeItem(PREFIX + key);
    } catch {
      /* ignore */
    }
  },
};

const DB = 'anamnesis-theme';
const STORE = 'blobs';
let dbPromise = null;

function openDb() {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve) => {
    try {
      if (!globalThis.indexedDB) return resolve(null);
      const req = indexedDB.open(DB, 1);
      req.onupgradeneeded = () => req.result.createObjectStore(STORE);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
  return dbPromise;
}

async function tx(mode, fn) {
  const db = await openDb();
  if (!db) return undefined;
  return new Promise((resolve) => {
    try {
      const t = db.transaction(STORE, mode);
      const req = fn(t.objectStore(STORE));
      t.oncomplete = () => resolve(req?.result);
      t.onerror = () => resolve(undefined);
      t.onabort = () => resolve(undefined);
    } catch {
      resolve(undefined);
    }
  });
}

const blobMemory = new Map();

export const blobs = {
  /** stores a Blob/File under key; returns true if it will survive a reload */
  async put(key, blob) {
    blobMemory.set(key, blob);
    const ok = (await tx('readwrite', (s) => s.put(blob, key))) !== undefined;
    if (ok) return true;
    // no IndexedDB: keep small files as data urls in localStorage
    if (blob.size <= 256 * 1024) {
      try {
        const url = await blobToDataUrl(blob);
        return kv.set('blob.' + key, url);
      } catch {
        return false;
      }
    }
    return false;
  },
  async get(key) {
    if (blobMemory.has(key)) return blobMemory.get(key);
    const b = await tx('readonly', (s) => s.get(key));
    if (b) return b;
    const url = kv.get('blob.' + key);
    if (url) {
      try {
        return await (await fetch(url)).blob();
      } catch {
        return null;
      }
    }
    return null;
  },
  async delete(key) {
    blobMemory.delete(key);
    kv.remove('blob.' + key);
    await tx('readwrite', (s) => s.delete(key));
  },
};

export function blobToDataUrl(blob) {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(r.result);
    r.onerror = () => reject(r.error);
    r.readAsDataURL(blob);
  });
}
