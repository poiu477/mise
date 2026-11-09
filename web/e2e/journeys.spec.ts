// The household's journeys through mise in a real browser, on the demo recipes (fixtures/demo-bundle), from an
// empty install: first-run setup, cooking at any serving count, planning a week, shopping (offline too), phone
// layout, accessibility and signing out. They run in order and share one browser session, as a person would.
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type BrowserContext, type Locator, type Page } from "@playwright/test";

test.describe.configure({ mode: "serial" });

let context: BrowserContext;
let page: Page;

test.beforeAll(async ({ browser }) => {
  context = await browser.newContext();
  page = await context.newPage();
});

test.afterAll(async () => {
  await context.close();
});

/** The amount shown next to an ingredient on a dish page. */
const amountOf = (name: string) =>
  page
    .getByRole("region", { name: "Ingredients" })
    .getByRole("listitem")
    .filter({ hasText: name })
    .locator(".amount");

/** A row on the shopping list, found by the food's name. */
const shoppingItem = (name: string): Locator =>
  page.locator(".shop-section li").filter({ has: page.locator(".item-name", { hasText: name }) });

test("the first visit creates the admin account", async () => {
  await page.goto("/");
  await expect(page).toHaveURL(/\/setup$/);
  await expect(page.getByRole("heading", { name: "Welcome to mise" })).toBeVisible();
  await page.getByLabel("Your name").fill("Robin");
  await page.getByLabel("Username").fill("robin");
  await page.getByLabel("Password", { exact: false }).first().fill("a long enough password");
  await page.getByLabel("Password again").fill("a long enough password");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
});

test("a dish reads as published, and scales to any serving count", async () => {
  await page.getByRole("searchbox", { name: "Search dishes" }).fill("tomato pasta");
  await page.getByRole("searchbox", { name: "Search dishes" }).press("Enter");
  await page.getByRole("link", { name: /Weeknight Tomato Pasta/ }).click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Weeknight Tomato Pasta");

  // 2 servings is published: amounts exactly as written, pinches as pinches.
  await expect(amountOf("Spaghetti")).toHaveText("6 oz");
  await expect(amountOf("Red Pepper Flakes")).toHaveText("1 pinch");
  await expect(amountOf("Yellow Onion")).toHaveText("1 onion");

  // 3 isn't published: scaled, rounded up, and marked.
  await page.getByRole("button", { name: "More servings" }).click();
  await expect(page.getByText("3 servings isn't a published size")).toBeVisible();
  await expect(amountOf("Spaghetti")).toHaveText("≈9 oz");
  await expect(amountOf("Garlic")).toHaveText("≈3 cloves");

  // 4 is published again: exact, no marks.
  await page.getByRole("button", { name: "More servings" }).click();
  await expect(amountOf("Spaghetti")).toHaveText("12 oz");
  await expect(page.getByText("isn't a published size")).toHaveCount(0);

  // Back to 2, and onto this week's plan.
  await page.getByRole("region", { name: "Servings" }).getByRole("button", { name: "2" }).click();
  await page.getByRole("button", { name: "Add to plan" }).click();
  await expect(page.getByRole("button", { name: "Added" })).toBeDisabled();
});

test("two more dishes and the week's list take under 20 interactions", async () => {
  // The Phase 3 gate: from a plan with one dish to the full list, counting every click and keystroke burst.
  let interactions = 0;
  const act = async (step: () => Promise<unknown>) => {
    interactions++;
    await step();
  };
  await act(() =>
    page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Plan" }).click(),
  );
  for (const dish of ["Black Bean Tacos", "Lemon Herb Chicken"]) {
    await act(() =>
      page.getByRole("searchbox", { name: "Search your dishes" }).fill(dish.split(" ")[0]),
    );
    await act(() => page.getByRole("button", { name: `Add ${dish}` }).click());
    await expect(page.locator(".plan-list").getByText(dish)).toBeVisible();
  }
  await act(() => page.getByRole("link", { name: "Shopping list" }).click());
  await expect(page.getByRole("heading", { name: "Produce" })).toBeVisible();
  expect(interactions).toBeLessThan(20);

  // One line per food, added up exactly across the three recipes at 2 servings each.
  await expect(shoppingItem("Garlic").locator(".item-amount")).toHaveText("3 cloves"); // 2 + 1
  await expect(shoppingItem("Yellow Onion").locator(".item-amount")).toHaveText("1½ onions"); // 1 + ½
  await expect(shoppingItem("Black Beans").locator(".item-amount")).toHaveText("1 can");
  // Pantry lines are listed to check, not to buy: 1 TBSP + 1 TBSP + 1 tsp of oil.
  const check = page.locator(".shop-section").filter({
    has: page.getByRole("heading", { name: "Check you have" }),
  });
  await expect(check.locator("li").filter({ hasText: "Olive Oil" })).toContainText("2⅓ TBSP");
  // Store sections in the order a shop is walked.
  const sections = await page.locator(".shop-section h2").allTextContents();
  expect(sections.indexOf("Produce")).toBeLessThan(sections.indexOf("Dairy & Eggs"));
  expect(sections.at(-1)).toBe("Check you have");
});

test("ticks survive a reload, and the list keeps working offline", async () => {
  const garlic = shoppingItem("Garlic").getByRole("checkbox");
  await garlic.check();
  await page.reload();
  await expect(shoppingItem("Garlic").getByRole("checkbox")).toBeChecked();

  // The service worker keeps the app itself, so it can open with no connection at all.
  await page.evaluate(async () => {
    await navigator.serviceWorker.ready;
  });
  if (!(await page.evaluate(() => !!navigator.serviceWorker.controller))) await page.reload();
  expect(await page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);

  await context.setOffline(true);
  await shoppingItem("Spaghetti").getByRole("checkbox").check();
  await page.getByRole("textbox", { name: "Add something else" }).fill("Foil");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Offline");
  await expect(page.getByText("not sent yet")).toBeVisible();

  // Reopening with no signal: the app loads from the service worker, the list from the phone's copy, and the
  // waiting changes are still on it.
  await page.reload();
  await expect(page.getByText("Offline: showing what this device has saved.")).toBeVisible();
  await expect(shoppingItem("Spaghetti").getByRole("checkbox")).toBeChecked();
  await expect(page.locator(".shop-section li").filter({ hasText: "Foil" })).toBeVisible();

  // Back online, the waiting changes are sent, once each.
  await context.setOffline(false);
  await expect(page.getByText("not sent yet")).toHaveCount(0, { timeout: 15_000 });
  await expect(page.getByRole("status")).toHaveCount(0);
  const list = await (await page.request.get("/api/shopping")).json();
  const items = list.sections.flatMap(
    (s: { items: { name: string; checked: boolean }[] }) => s.items,
  );
  expect(items.find((i: { name: string }) => i.name === "Spaghetti").checked).toBe(true);
  expect(items.find((i: { name: string }) => i.name === "Garlic").checked).toBe(true);
  expect(list.extras.map((x: { name: string }) => x.name)).toEqual(["Foil"]);
});

test("an addition whose answer is lost is sent again and added once", async () => {
  // The server saves the first send, but the answer never reaches the phone, so the addition stays waiting.
  let sends = 0;
  let firstSaved = () => {};
  const savedOnce = new Promise<void>((resolve) => (firstSaved = resolve));
  await page.route("**/api/shopping/extras", async (r) => {
    sends++;
    if (sends > 1) return r.continue();
    await r.fetch();
    firstSaved();
    await r.abort();
  });
  await page.getByRole("textbox", { name: "Add something else" }).fill("Tape");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  // "not sent yet" shows from the moment the addition is queued, so it can't tell us the server has it: wait for
  // the first send to have been saved before reading the server's list.
  await savedOnce;
  await expect(page.getByText("not sent yet")).toBeVisible();
  const saved = await (await page.request.get("/api/shopping")).json();
  expect(saved.extras.map((x: { name: string }) => x.name)).toEqual(["Foil", "Tape"]);

  // Reopening sends it again, and the server knows it already.
  await page.reload();
  await expect.poll(() => sends, { timeout: 15_000 }).toBe(2);
  await expect(page.getByText("not sent yet")).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: "Tape", exact: true })).toHaveCount(1);
  const list = await (await page.request.get("/api/shopping")).json();
  expect(list.extras.map((x: { name: string }) => x.name)).toEqual(["Foil", "Tape"]);
  await page.unroute("**/api/shopping/extras");
});

test("a list left open in one tab keeps saving after another tab signs in", async () => {
  await page.goto("/shopping");
  await expect(shoppingItem("Black Beans")).toBeVisible();
  // Another tab signs in again (as after a session that expired), which starts a new sign-in on this device.
  const other = await context.newPage();
  await other.goto("/account");
  await other.evaluate(() => localStorage.setItem("mise:sign-in", `other-tab-${Date.now()}`));
  await other.close();
  // The first tab, still open, ticks something offline; it must survive a reload.
  await context.setOffline(true);
  await shoppingItem("Black Beans").getByRole("checkbox").check();
  await expect(page.getByRole("status")).toContainText("Offline");
  await page.reload();
  await expect(shoppingItem("Black Beans").getByRole("checkbox")).toBeChecked();
  await context.setOffline(false);
  await expect(page.getByRole("status")).toHaveCount(0, { timeout: 15_000 });
});

test("work queued as one person never lands on whoever signs in after them", async () => {
  // Robin, the admin, adds Sam, in a household of Sam's own.
  const made = await page.request.post("/api/users", {
    data: { username: "sam", password: "sam's long password", household_name: "Sam's household" },
  });
  expect(made.ok()).toBe(true);
  await page.goto("/shopping");
  await expect(shoppingItem("Black Beans")).toBeVisible();

  // Robin adds something the server can't take just now, so it waits on the device.
  await page.route("**/api/shopping/extras", (r) => r.abort());
  await page.getByRole("textbox", { name: "Add something else" }).fill("Robin's foil");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByText("not sent yet")).toBeVisible();
  await page.unroute("**/api/shopping/extras");

  // Sam signs in, in another tab: the cookie is shared, so every tab now talks to the server as Sam.
  const other = await context.newPage();
  await other.request.post("/api/auth/logout");
  const login = await other.request.post("/api/auth/login", {
    data: { username: "sam", password: "sam's long password" },
  });
  expect(login.ok()).toBe(true);
  await other.goto("/plan");
  await expect(other.getByText("Nothing planned yet")).toBeVisible();

  // Robin's tab notices the new sign-in and tries again, with its waiting work made as Robin: the server refuses
  // it as meant for someone else, and the tab follows Sam instead of sending it as Sam.
  await expect(page.getByText("Nothing planned for this week yet")).toBeVisible();
  await page.waitForLoadState("networkidle");
  const sams = await (await other.request.get("/api/shopping")).json();
  expect(sams.extras).toEqual([]);
  // Robin's addition is still waiting on the device, for Robin.
  const waiting = await page.evaluate(() =>
    Object.keys(localStorage)
      .filter((k) => k.endsWith(":pending"))
      .map((k) => localStorage.getItem(k))
      .join(""),
  );
  expect(waiting).toContain("Robin's foil");
  await other.close();
});

const pages = ["/", "/dishes/weeknight-tomato-pasta", "/plan", "/shopping", "/account"];

test("every page fits a phone without sideways scrolling", async () => {
  await page.setViewportSize({ width: 375, height: 812 });
  for (const path of pages) {
    await page.goto(path);
    await expect(page.locator("main")).toBeVisible();
    const [scroll, width] = await page.evaluate(() => [
      document.documentElement.scrollWidth,
      window.innerWidth,
    ]);
    expect(scroll, `${path} is ${scroll}px wide on a ${width}px screen`).toBeLessThanOrEqual(width);
  }
  await page.setViewportSize({ width: 1280, height: 800 });
});

test("pages have no serious accessibility problems", async () => {
  for (const path of pages) {
    await page.goto(path);
    // Scan the page as it rests: loaded, and not dimmed while a new version or serving count loads.
    await page.waitForLoadState("networkidle");
    await expect(page.locator(".stale")).toHaveCount(0);
    const { violations } = await new AxeBuilder({ page }).analyze();
    const serious = violations
      .filter((v) => v.impact === "serious" || v.impact === "critical")
      .flatMap((v) => v.nodes.map((n) => `${v.id}: ${n.target.join(" ")}: ${n.failureSummary}`));
    expect(serious, path).toEqual([]);
  }
});

test("signing out leaves nothing of the household on the device", async () => {
  await page.goto("/account");
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/login/);
  const kept = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.startsWith("mise:")),
  );
  expect(kept).toEqual([]);
});
