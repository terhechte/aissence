import { expect, test } from '@playwright/test';

test('home page presents the real publishing flow', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 })).toContainText('reasoning behind the commit');
  await expect(page.getByText('Drop the project record')).toBeVisible();
  await expect(page.getByText('No public ledgers yet.')).toBeVisible();
});

test('sign-in page explains missing local configuration', async ({ page }) => {
  await page.goto('/auth');
  await expect(page.getByRole('heading', { level: 1 })).toContainText('No password');
  await expect(page.getByText(/Copy .*\.env\.example/)).toBeVisible();
});

test('layout remains usable on a mobile viewport', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'Aissence home' })).toBeVisible();
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
});
