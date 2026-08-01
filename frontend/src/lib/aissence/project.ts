import { z } from 'zod';

export const projectMetadataSchema = z.object({
  name: z.string().trim().min(1).max(120),
  slug: z
    .string()
    .trim()
    .toLowerCase()
    .min(2)
    .max(80)
    .regex(/^[a-z0-9][a-z0-9-]+$/, 'Use lowercase letters, numbers, and hyphens'),
  tagline: z.string().trim().min(1).max(240),
  description: z.string().trim().min(1).max(3000),
  product_url: z
    .union([z.literal(''), z.string().url().startsWith('https://', 'Use an HTTPS URL')])
    .transform((value) => value || null),
  repository_label: z
    .string()
    .trim()
    .max(160)
    .optional()
    .transform((value) => value || null),
  repository_url: z
    .union([z.literal(''), z.string().url().startsWith('https://', 'Use an HTTPS URL')])
    .transform((value) => value || null),
  comment_visibility: z.enum(['public', 'subscribers']).default('subscribers')
});

export interface RepositoryFields {
  label: string;
  url: string;
}

/** Convert the repository reference stored in an .aissence file to form fields. */
export function repositoryFields(repository: string): RepositoryFields {
  const trimmed = repository.trim().replace(/\/+$/, '');
  const normalized = trimmed.replace(/^git@github\.com:/i, 'https://github.com/');
  const match = normalized.match(
    /^(?:https?:\/\/)?(?:www\.)?github\.com\/([^/\s]+)\/([^/?#\s]+?)(?:\.git)?\/?(?:[?#].*)?$/i
  );

  if (match) {
    const label = `${match[1]}/${match[2]}`;
    return { label, url: `https://github.com/${label}` };
  }

  return {
    label: trimmed,
    url: /^https:\/\//i.test(trimmed) ? trimmed : ''
  };
}

export const profileSchema = z.object({
  handle: z
    .string()
    .trim()
    .toLowerCase()
    .min(3)
    .max(40)
    .regex(/^[a-z0-9][a-z0-9-]+$/, 'Use lowercase letters, numbers, and hyphens'),
  display_name: z.string().trim().min(1).max(100),
  bio: z.string().trim().max(500).optional().transform((value) => value || null)
});

export const screenshotMetadataSchema = z.object({
  storage_path: z.string().min(1),
  alt_text: z
    .string()
    .trim()
    .max(300)
    .optional()
    .transform((value) => value || ''),
  caption: z.string().trim().max(300).optional().transform((value) => value || null),
  sort_order: z.number().int().min(0).max(7)
});

export type ProjectMetadata = z.infer<typeof projectMetadataSchema>;
