import { describe, expect, it } from 'vitest';
import {
  estimatePotentialCost,
  priceSnapshot,
  resolveOpenRouterModel,
  type OpenRouterModel
} from './openrouter-pricing';

const catalog: OpenRouterModel[] = [
  {
    id: 'openai/gpt-5.6-sol',
    pricing: {
      prompt: '0.000005',
      completion: '0.00003',
      input_cache_read: '0.0000005',
      input_cache_write: '0.00000625',
      overrides: [
        {
          min_prompt_tokens: 272_000,
          prompt: '0.00001',
          completion: '0.000045'
        }
      ]
    }
  },
  {
    id: 'anthropic/claude-opus-4.8',
    pricing: { prompt: '0.000005', completion: '0.000025' }
  },
  {
    id: 'anthropic/claude-opus-4.8:batch',
    pricing: { prompt: '0.0000025', completion: '0.0000125' }
  },
  {
    id: 'moonshotai/kimi-k3',
    pricing: { prompt: '0.000003', completion: '0.000015' }
  },
  {
    id: 'openai/gpt-5.6-luna',
    pricing: { prompt: '0.000004', completion: '0.00002' }
  },
  {
    id: 'openai/gpt-5.6-terra',
    pricing: { prompt: '0.000004', completion: '0.00002' }
  },
  {
    id: 'anthropic/claude-fable-5',
    pricing: { prompt: '0.000004', completion: '0.00002' }
  },
  {
    id: 'anthropic/claude-opus-5',
    pricing: { prompt: '0.000006', completion: '0.00003' }
  }
];

describe('OpenRouter pricing', () => {
  it.each([
    ['Codex', 'gpt-5.6-sol', 'openai/gpt-5.6-sol'],
    ['Codex', 'gpt-5.6-terra', 'openai/gpt-5.6-terra'],
    ['Codex', 'gpt-5.6-luna', 'openai/gpt-5.6-luna'],
    ['Claude Code', 'claude-opus-4-8', 'anthropic/claude-opus-4.8'],
    ['Claude Code', 'claude-fable-5', 'anthropic/claude-fable-5'],
    ['Claude Code', 'claude-opus-5', 'anthropic/claude-opus-5'],
    ['Kimi CLI', 'kimi-code/k3', 'moonshotai/kimi-k3']
  ])('matches %s model identifiers', (provider, model, expected) => {
    expect(resolveOpenRouterModel({ provider, model }, catalog)?.id).toBe(expected);
  });

  it('uses the highest matching model estimate and cache rates', () => {
    const cost = estimatePotentialCost(
      { input: 10_000, output: 1_000, cache_read: 8_000, cache_create: 500 },
      [
        { provider: 'Codex', model: 'gpt-5.6-sol' },
        { provider: 'Kimi CLI', model: 'kimi-code/k3' }
      ],
      catalog,
      '2026-07-30T10:00:00.000Z'
    );

    expect(cost.input_cost_usd).toBe(0.057125);
    expect(cost.output_cost_usd).toBe(0.03);
    expect(cost.potential_cost_usd).toBe(0.087125);
    expect(cost.model_ids).toEqual(['openai/gpt-5.6-sol', 'moonshotai/kimi-k3']);
    expect(cost.pricing_method).toBe('highest_rate_fallback');
  });

  it('prices each model from its own token usage and sums the costs', () => {
    const cost = estimatePotentialCost(
      { input: 15_000, output: 1_500, cache_read: 9_000, cache_create: 500 },
      [
        {
          provider: 'Codex',
          model: 'gpt-5.6-sol',
          tokens: {
            input: 10_000,
            output: 1_000,
            cache_read: 8_000,
            cache_create: 500
          }
        },
        {
          provider: 'Kimi CLI',
          model: 'kimi-code/k3',
          tokens: { input: 5_000, output: 500, cache_read: 1_000 }
        }
      ],
      catalog,
      '2026-07-30T10:00:00.000Z'
    );

    expect(cost.input_cost_usd).toBe(0.075125);
    expect(cost.output_cost_usd).toBe(0.0375);
    expect(cost.potential_cost_usd).toBe(0.112625);
    expect(cost.pricing_method).toBe('per_agent');
  });

  it('falls back when per-agent usage is incomplete or does not match the commit total', () => {
    const cost = estimatePotentialCost(
      { input: 15_000, output: 1_500 },
      [
        {
          provider: 'Codex',
          model: 'gpt-5.6-sol',
          tokens: { input: 10_000, output: 1_000 }
        },
        { provider: 'Kimi CLI', model: 'kimi-code/k3' }
      ],
      catalog
    );

    expect(cost.input_cost_usd).toBe(0.075);
    expect(cost.output_cost_usd).toBe(0.045);
    expect(cost.pricing_method).toBe('highest_rate_fallback');
  });

  it('limits conservative fallback pricing to an unmatched agent subtotal', () => {
    const cost = estimatePotentialCost(
      { input: 20_000, output: 2_000 },
      [
        {
          provider: 'Kimi CLI',
          model: 'kimi-code/k3',
          tokens: { input: 5_000, output: 500 }
        },
        {
          provider: 'Claude Code',
          model: '<synthetic>',
          tokens: { input: 5_000, output: 500 }
        },
        {
          provider: 'Codex',
          model: 'gpt-5.6-sol',
          tokens: { input: 10_000, output: 1_000 }
        }
      ],
      catalog
    );

    expect(cost.input_cost_usd).toBe(0.09);
    expect(cost.output_cost_usd).toBe(0.0525);
    expect(cost.potential_cost_usd).toBe(0.1425);
    expect(cost.pricing_method).toBe('per_agent_with_fallback');
  });

  it('applies long-context pricing overrides', () => {
    const cost = estimatePotentialCost(
      { input: 300_000, output: 2_000 },
      [{ provider: 'Codex', model: 'gpt-5.6-sol' }],
      catalog
    );
    expect(cost.input_cost_usd).toBe(3);
    expect(cost.output_cost_usd).toBe(0.09);
  });

  it('keeps unmatched pricing explicitly unavailable', () => {
    const snapshot = priceSnapshot(
      {
        commits: [
          {
            commit_hash: '8f2a1c4e9b7d6a5f0c3e2b1a4d8c7e6f9a0b1c2d',
            source_digest: `sha256:${'0'.repeat(64)}`,
            sanitizer_version: 1,
            title: 'Unknown model',
            date: '2026-07-30T10:00:00Z',
            stats: { files_changed: 0, lines_added: 0, lines_deleted: 0 },
            files: [],
            source: {
              messages: [],
              agents: [{ provider: 'Mystery', model: 'unlisted-model' }],
              tokens: { input: 10, output: 2 }
            }
          }
        ]
      },
      catalog
    );
    expect(snapshot.commits[0].pricing.potential_cost_usd).toBeNull();
  });
});
