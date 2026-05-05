const TOKEN_KEY = "stackchan-auth-token";

export function getAuthToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? "";
}

export function setAuthToken(token: string): void {
  if (token) {
    localStorage.setItem(TOKEN_KEY, token);
  } else {
    localStorage.removeItem(TOKEN_KEY);
  }
}

function authHeaders(extra: HeadersInit = {}): HeadersInit {
  const token = getAuthToken();
  if (!token) return extra;
  return { ...extra, Authorization: `Bearer ${token}` };
}

export async function authedFetch(
  path: string,
  init: RequestInit = {},
): Promise<Response> {
  let res = await fetch(path, { ...init, headers: authHeaders(init.headers) });
  if (res.status === 401) {
    const next = prompt("Auth token required. Enter Bearer token:", getAuthToken());
    if (next != null) {
      setAuthToken(next);
      res = await fetch(path, { ...init, headers: authHeaders(init.headers) });
    }
  }
  return res;
}

export async function postJson(path: string, body: unknown): Promise<void> {
  const res = await authedFetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body == null ? "" : JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`${res.status}: ${text || res.statusText}`);
  }
}

export async function putJson(path: string, body: unknown): Promise<void> {
  const res = await authedFetch(path, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`${res.status}: ${text || res.statusText}`);
  }
}
