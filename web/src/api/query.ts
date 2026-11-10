// Server data for the pages, through TanStack Query: one cache for the app, and the rules every query shares.
// Keys start with what the data is, then the account it belongs to when it is an account's own (plan, list,
// settings), so one person's data never shows for whoever signs in next.
import { onlineManager, QueryClient } from "@tanstack/react-query";
import { errorCode, errorText, type ErrorCode } from "./client";

/** An answer from the server that isn't the data asked for, with its status and the API's error `code` (a
 * `not_found` reads differently). */
export class Refused extends Error {
  readonly status: number;
  readonly body: unknown;
  /** The API's name for the error; null when something other than mise answered, such as a proxy. */
  readonly code: ErrorCode | null;
  constructor(status: number, body: unknown) {
    super(errorText(body));
    this.status = status;
    this.body = body;
    this.code = errorCode(body);
  }
}

/** Whether the server refused what was asked (a 4xx, such as a serving count out of range), as opposed to failing
 * itself: a 5xx, or a proxy's 502 while mise restarts, is worth trying again. */
export function refusedRequest(error: unknown): error is Refused {
  return error instanceof Refused && error.status < 500;
}

/** The data an API call answered with. Throws `Refused` for any other answer; a call that gets no answer at all
 * throws as `fetch` does. */
export async function answer<T>(
  call: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  const { data, error, response } = await call;
  if (data === undefined) throw new Refused(response.status, error);
  return data;
}

// TanStack assumes it starts online and reloads on the browser's "online" event only when it saw itself go
// offline. An app opened with no signal starts offline, so coming back into reach reloads what is on screen.
if (typeof navigator !== "undefined") onlineManager.setOnline(navigator.onLine);

/** The app's one cache of server data, shared by every page. */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Always ask, even when the browser says it's offline: the service worker answers recipe requests from its
      // cache, and the shopping list falls back to the device's copy.
      networkMode: "always",
      // "always" turns reloading on reconnect off unless asked for; the list needs it to send what waited offline.
      refetchOnReconnect: true,
      // A page says at once when something didn't load, as it always has; asking again wouldn't change a refusal.
      retry: false,
    },
  },
});
