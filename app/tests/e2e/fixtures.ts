import { expect, test as base, type Page } from "@playwright/test";

/** Fails the test on any uncaught page error or console error. */
export const test = base.extend<{ app: Page }>({
  app: async ({ page }, use) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    await use(page);
    expect(errors, "page errors").toEqual([]);
  },
});

export { expect };

/** Open the app on the mock backend. */
export async function open(page: Page, query = "delay=0") {
  await page.goto(`/?${query}`);
  await expect(page.getByTestId("sidebar").or(page.getByTestId("init-error"))).toBeVisible();
}

export async function setApiKey(page: Page) {
  await page.getByTestId("nav-settings").click();
  await page.getByLabel("ANTHROPIC_API_KEY").fill("sk-e2e");
  await page.getByTestId("save-settings").click();
  await expect(page.getByTestId("toast-success").filter({ hasText: "Settings saved" })).toBeVisible();
}
