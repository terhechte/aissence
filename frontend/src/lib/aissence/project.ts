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
  product_url: z.string().url().startsWith('https://', 'Use an HTTPS URL'),
  repository_label: z
    .string()
    .trim()
    .max(160)
    .optional()
    .transform((value) => value || null),
  repository_url: z
    .union([z.literal(''), z.string().url().startsWith('https://', 'Use an HTTPS URL')])
    .transform((value) => value || null)
});

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
