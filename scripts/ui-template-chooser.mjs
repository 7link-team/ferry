/** Explicitly choose an empty draft at creation; never bypass the chooser.
 *  Resource forms open the chooser as a dialog; application creation shows it as a page section. */
export async function startBlank(page) {
  const dialog = page.getByRole('dialog').filter({ has: page.getByRole('heading', { name: /^(选择创建方式|Choose how to start)$/ }) });
  const chooser = dialog.or(page.locator('section[data-template-selection]'));
  await chooser.first().waitFor();
  const scope = (await dialog.count()) ? dialog : page.locator('section[data-template-selection]');
  await scope.getByRole('button', { name: /^(空白创建|Start blank)$/ }).click();
  await scope.waitFor({ state: 'hidden' });
}
