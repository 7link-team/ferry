/** Explicitly choose an empty draft at creation; never bypass the chooser. */
export async function startBlank(page) {
  const dialog = page.getByRole('dialog').filter({ has: page.getByRole('heading', { name: /^(选择创建方式|Choose how to start)$/ }) });
  await dialog.waitFor();
  await dialog.getByRole('button', { name: /^(空白创建|Start blank)$/ }).click();
  await dialog.waitFor({ state: 'hidden' });
}
