export function fetchJSON(path) {
  return fetch(path, { cache: "no-store" }).then((resp) => {
    if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
    return resp.json();
  });
}

// sendJSON covers POST/PUT bodies (config pushes, tag edits) -- reads the
// body as JSON even on a non-2xx response, since this server's error
// responses are themselves JSON ({"error": "..."}), not plain text.
export function sendJSON(method, path, body) {
  return fetch(path, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).then(async (resp) => {
    const data = await resp.json().catch(() => null);
    if (!resp.ok) {
      const message = data?.error || `${resp.status} ${resp.statusText}`;
      throw new Error(message);
    }
    return data;
  });
}
