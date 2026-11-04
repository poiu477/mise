import { describe, expect, it } from "vitest";
import type { ShoppingList } from "./api/client";
import {
  deviceCopy,
  forgetDevice,
  queue,
  rememberedMe,
  rememberMe,
  sendChanges,
  startSignIn,
  withChanges,
  type Change,
  type Sent,
  type Store,
} from "./offline";

function memory(): Store & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    get: (k) => data.get(k) ?? null,
    set: (k, v) => void data.set(k, v),
    remove: (k) => void data.delete(k),
    keys: () => [...data.keys()],
  };
}

const WEEK = "2026-11-02";
const list = (over: Partial<ShoppingList> = {}): ShoppingList => ({
  week: WEEK,
  meals: 1,
  weight_unit: "lb_oz",
  removed: [],
  staples: [],
  extras: [{ id: "x1", name: "Paper towels", checked: false }],
  sections: [
    {
      name: "Produce",
      items: [
        {
          key: "onion",
          food_id: null,
          name: "Onion",
          amount: "2 whole",
          checked: false,
          recipes: ["Chili"],
          weighed: false,
        },
        {
          key: "lime",
          food_id: null,
          name: "Lime",
          amount: "1 whole",
          checked: true,
          recipes: ["Chili"],
          weighed: false,
        },
      ],
    },
  ],
  ...over,
});
const tick = (key: string, checked: boolean, week = WEEK): Change => ({
  kind: "tick",
  week,
  key,
  checked,
});

/** Signed in on this device as `id`, as the session does after `/api/me` answers. */
const signedIn = (store: Store, id = "me") =>
  rememberMe(
    {
      id,
      username: "paul",
      display_name: "Paul",
      email: null,
      is_admin: true,
      has_password: true,
      household: { id: "h", name: "Home" },
    },
    store,
  );

describe("the device copy", () => {
  it("keeps each week's list and the waiting changes, per account", () => {
    const store = memory();
    signedIn(store);
    const mine = deviceCopy("me", store);
    mine.save(list(), 1000);
    expect(mine.saved(WEEK)).toEqual({ list: list(), savedAt: 1000 });
    expect(mine.saved("2026-11-09")).toBeNull();
    expect(deviceCopy("someone else", store).saved(WEEK)).toBeNull();
    mine.setPending([tick("onion", true)]);
    expect(mine.pending()).toEqual([tick("onion", true)]);
    mine.setPending([]);
    expect(mine.pending()).toEqual([]);
  });

  it("is forgotten on signing out, along with who was signed in", () => {
    const store = memory();
    store.set("other-app", "kept");
    signedIn(store);
    deviceCopy("me", store).save(list());
    expect(rememberedMe(store)?.username).toBe("paul");
    forgetDevice(store);
    expect([...store.data.keys()]).toEqual(["other-app"]);
    expect(rememberedMe(store)).toBeNull();
  });

  it("saves nothing once the account has signed out, even from an answer still on its way", () => {
    const store = memory();
    signedIn(store);
    const mine = deviceCopy("me", store);
    forgetDevice(store);
    // A list that was being fetched when the person signed out arrives now.
    mine.save(list());
    mine.setPending([tick("onion", true)]);
    expect([...store.data.keys()]).toEqual([]);
    // Nor for an account that isn't the one signed in here.
    signedIn(store, "someone else");
    mine.save(list());
    expect(mine.saved(WEEK)).toBeNull();
  });

  it("can't touch a later sign-in's copy, even for the same account", () => {
    const store = memory();
    signedIn(store);
    const before = deviceCopy("me", store);
    // Signed out and straight back in as the same account, with an answer from before still on its way.
    forgetDevice(store);
    signedIn(store);
    const now = deviceCopy("me", store);
    now.setPending([tick("onion", true)]);
    before.save(list());
    before.setPending([]);
    expect(now.saved(WEEK)).toBeNull();
    expect(now.pending()).toEqual([tick("onion", true)]);
    // Checking who is signed in again (every time the app opens) keeps the same sign-in.
    signedIn(store);
    now.save(list());
    expect(now.saved(WEEK)?.list).toEqual(list());
  });

  it("starts a new sign-in when the same account signs in again after its session ended", () => {
    const store = memory();
    signedIn(store);
    const before = deviceCopy("me", store);
    // The session expired (nothing was cleared), and the same person signs in again on the sign-in page.
    startSignIn(store);
    signedIn(store);
    const now = deviceCopy("me", store);
    now.setPending([tick("onion", true)]);
    before.setPending([]);
    before.save(list());
    expect(now.pending()).toEqual([tick("onion", true)]);
    expect(now.saved(WEEK)).toBeNull();
  });

  it("keeps working on a device remembered before sign-ins had ids", () => {
    const store = memory();
    signedIn(store);
    store.remove("mise:sign-in");
    // Opened offline, so no fresh sign-in: the remembered account's copy still saves ticks.
    const mine = deviceCopy("me", store);
    mine.setPending([tick("onion", true)]);
    expect(mine.pending()).toEqual([tick("onion", true)]);
    // But never once signed out.
    forgetDevice(store);
    deviceCopy("me", store).save(list());
    expect([...store.data.keys()]).toEqual([]);
  });

  it("reads a damaged copy as none", () => {
    const store = memory();
    store.set("mise:me:list:2026-11-02", "{not json");
    expect(deviceCopy("me", store).saved(WEEK)).toBeNull();
  });
});

describe("waiting changes", () => {
  it("keep only the latest tick of each thing", () => {
    let q = queue([], tick("onion", true));
    q = queue(q, tick("lime", false));
    q = queue(q, tick("onion", false));
    expect(q).toEqual([tick("lime", false), tick("onion", false)]);
    // The same key in another week is another item.
    expect(queue(q, tick("onion", true, "2026-11-09"))).toHaveLength(3);
  });

  it("tick something added here as it is sent", () => {
    const add: Change = {
      kind: "add_extra",
      week: WEEK,
      id: "local-1",
      name: "Foil",
      checked: false,
    };
    expect(queue([add], { kind: "tick_extra", week: WEEK, id: "local-1", checked: true })).toEqual([
      { ...add, checked: true },
    ]);
  });

  it("show on the list", () => {
    const shown = withChanges(list(), [
      tick("onion", true),
      tick("lime", false),
      { kind: "tick_extra", week: WEEK, id: "x1", checked: true },
      { kind: "add_extra", week: WEEK, id: "local-1", name: "Foil", checked: false },
      tick("onion", false, "2026-11-09"),
    ]);
    expect(shown.sections[0].items.map((i) => i.checked)).toEqual([true, false]);
    expect(shown.extras).toEqual([
      { id: "x1", name: "Paper towels", checked: true },
      { id: "local-1", name: "Foil", checked: false },
    ]);
    expect(withChanges(list(), [])).toEqual(list());
  });
});

describe("sending", () => {
  it("goes in order and stops where the server can't be reached", async () => {
    const sent: Change[] = [];
    const answers: Sent[] = [{ list: list({ meals: 2 }) }, "refused", "unreachable"];
    const changes = [
      tick("onion", true),
      tick("gone", true),
      tick("lime", false),
      tick("onion", false),
    ];
    const r = await sendChanges(changes, async (c) => {
      sent.push(c);
      return answers.shift()!;
    });
    expect(sent).toEqual(changes.slice(0, 3));
    expect(r.remaining).toEqual(changes.slice(2));
    expect(r.refused).toBe(1);
    expect(r.list?.meals).toBe(2);
  });

  it("carries on with what is left of a change", async () => {
    const add: Change = {
      kind: "add_extra",
      week: WEEK,
      id: "local-1",
      name: "Foil",
      checked: true,
    };
    const then: Change = { kind: "tick_extra", week: WEEK, id: "x9", checked: true };
    const seen: Change[] = [];
    const r = await sendChanges([add], async (c) => {
      seen.push(c);
      return c === add ? { list: list(), then } : "unreachable";
    });
    // The addition went through; only the tick is still to send, so it isn't added twice.
    expect(seen).toEqual([add, then]);
    expect(r.remaining).toEqual([then]);
  });
});
