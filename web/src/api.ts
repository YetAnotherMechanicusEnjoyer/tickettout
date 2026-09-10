export class ApiError extends Error {
  status: number

  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

export async function api<T>(
  path: string,
  options?: RequestInit,
): Promise<T> {
  const response = await fetch(`/api${path}`, options);

  if (!response.ok) {
    const body = await response.text().catch(() => "");
    let message = body;
    try {
      const parsed = JSON.parse(body);
      if (parsed && typeof parsed.error === "string") {
        message = parsed.error;
      }
    } catch {
      /* body isn't JSON, use it as-is */
    }
    throw new ApiError(
      response.status,
      message || `API request failed: ${response.status}`,
    );
  }

  return response.json() as Promise<T>;
}

// e.g.: const health = await api<{ status: string }>("/health");
// const url = await api<{ url: string }>("api_url sans api au début")
