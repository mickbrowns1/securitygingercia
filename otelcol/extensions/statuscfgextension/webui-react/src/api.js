export function fetchJSON(path) {
  return fetch(path, { cache: "no-store" }).then((resp) => {
    if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
    return resp.json();
  });
}
