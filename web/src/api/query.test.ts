import { describe, expect, it } from "vitest";
import { Refused, refusedRequest } from "./query";

describe("refusedRequest", () => {
  it("is a 4xx answer, which reads as the server's own reason", () => {
    const e = new Refused(400, { error: "servings must be more than 0 and at most 24" });
    expect(refusedRequest(e)).toBe(true);
    expect(e.message).toBe("Servings must be more than 0 and at most 24.");
    expect(refusedRequest(new Refused(404, null))).toBe(true);
  });
  it("isn't a server's failure or no answer at all, which are worth trying again", () => {
    expect(refusedRequest(new Refused(500, null))).toBe(false);
    expect(refusedRequest(new Refused(502, "Bad Gateway"))).toBe(false);
    expect(refusedRequest(new TypeError("Failed to fetch"))).toBe(false);
    expect(refusedRequest(null)).toBe(false);
  });
});
