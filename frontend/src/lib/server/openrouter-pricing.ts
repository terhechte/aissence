import type { Agent, NormalizedSnapshot, TokenCount } from '$lib/aissence/schema';

const OPENROUTER_MODELS_URL = 'https://openrouter.ai/api/v1/models';
const CATALOG_TTL_MS = 60 * 60 * 1000;

type PricingValue = string | number | undefined;

export interface OpenRouterPricing {
  prompt?: PricingValue;
  completion?: PricingValue;
  input_cache_read?: PricingValue;
  input_cache_write?: PricingValue;
  overrides?: Array<{
    min_prompt_tokens?: number;
    prompt?: PricingValue;
    completion?: PricingValue;
    input_cache_read?: PricingValue;
    input_cache_write?: PricingValue;
  }>;
}

export interface OpenRouterModel {
  id: string;
  pricing: OpenRouterPricing;
}

export interface PotentialCost {
  input_cost_usd: number | null;
  output_cost_usd: number | null;
  potential_cost_usd: number | null;
  model_ids: string[];
  priced_at: string;
  pricing_method:
    | 'per_agent'
    | 'per_agent_with_fallback'
    | 'highest_rate_fallback'
    | 'unavailable';
}

let cachedCatalog: { expiresAt: number; models: OpenRouterModel[] } | null = null;

const providerAuthors: Array<[RegExp, string]> = [
  [/\bcodex\b|\bopenai\b/i, 'openai'],
  [/\bclaude\b|\banthropic\b/i, 'anthropic'],
  [/\bkimi\b|\bmoonshot\b/i, 'moonshotai'],
  [/\bgemini\b|\bantigravity\b|\bgoogle\b/i, 'google']
];

function finitePrice(value: PricingValue): number | null {
  const parsed = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

function normalizeIdentifier(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, '');
}

function identifierVariants(value: string): Set<string> {
  const withoutVariant = value.replace(/:[a-z0-9-]+$/i, '');
  return new Set([
    normalizeIdentifier(withoutVariant),
    normalizeIdentifier(withoutVariant.replace(/(^|[./_-])(code|cli)(?=($|[./_-]))/gi, '$1'))
  ]);
}

function providerAuthor(provider: string): string | null {
  return providerAuthors.find(([pattern]) => pattern.test(provider))?.[1] ?? null;
}

export function resolveOpenRouterModel(
  agent: Agent,
  catalog: OpenRouterModel[]
): OpenRouterModel | null {
  if (!agent.model) return null;
  const rawModel = agent.model.toLowerCase().replace(/^~+/, '');
  const exact = catalog.find(
    (model) => model.id.toLowerCase().replace(/^~+/, '') === rawModel
  );
  if (exact) return exact;

  const expectedAuthor = providerAuthor(agent.provider);
  const authoredExact = expectedAuthor
    ? catalog.find(
        (model) =>
          model.id.toLowerCase().replace(/^~+/, '') === `${expectedAuthor}/${rawModel}`
      )
    : null;
  if (authoredExact) return authoredExact;

  const variants = identifierVariants(rawModel);
  const matches = catalog.filter((model) => {
    const normalizedId = model.id.toLowerCase().replace(/^~+/, '');
    if (!rawModel.includes(':') && normalizedId.includes(':')) return false;
    const [author, ...slugParts] = normalizedId.split('/');
    if (expectedAuthor && author !== expectedAuthor) return false;
    const slug = slugParts.join('/');
    return [...identifierVariants(slug)].some((candidate) => variants.has(candidate));
  });

  return matches.length === 1 ? matches[0] : null;
}

function effectivePricing(pricing: OpenRouterPricing, inputTokens: number): OpenRouterPricing {
  const override = [...(pricing.overrides ?? [])]
    .filter(
      (candidate) =>
        Number.isFinite(candidate.min_prompt_tokens) &&
        Number(candidate.min_prompt_tokens) <= inputTokens
    )
    .sort((left, right) => Number(right.min_prompt_tokens) - Number(left.min_prompt_tokens))[0];
  return override ? { ...pricing, ...override, overrides: pricing.overrides } : pricing;
}

function roundUsd(value: number): number {
  return Math.round((value + Number.EPSILON) * 100_000_000) / 100_000_000;
}

interface ModelEstimate {
  model: OpenRouterModel;
  inputCost: number;
  outputCost: number;
  totalCost: number;
}

function estimateModelCost(
  tokens: TokenCount,
  model: OpenRouterModel
): ModelEstimate | null {
  const pricing = effectivePricing(model.pricing, tokens.input);
  const prompt = finitePrice(pricing.prompt);
  const completion = finitePrice(pricing.completion);
  if (prompt === null || completion === null) return null;

  const cacheRead = finitePrice(pricing.input_cache_read) ?? prompt;
  const cacheWrite = finitePrice(pricing.input_cache_write) ?? prompt;
  const inputCost =
    tokens.input * prompt +
    (tokens.cache_read ?? 0) * cacheRead +
    (tokens.cache_create ?? 0) * cacheWrite;
  const outputCost = tokens.output * completion;
  return {
    model,
    inputCost,
    outputCost,
    totalCost: inputCost + outputCost
  };
}

function tokenCountsMatch(total: TokenCount, agents: Agent[]): boolean {
  if (agents.length === 0 || agents.some((agent) => !agent.tokens)) return false;
  const summed = agents.reduce(
    (sum, agent) => ({
      input: sum.input + agent.tokens!.input,
      output: sum.output + agent.tokens!.output,
      cache_read: sum.cache_read + (agent.tokens!.cache_read ?? 0),
      cache_create: sum.cache_create + (agent.tokens!.cache_create ?? 0)
    }),
    { input: 0, output: 0, cache_read: 0, cache_create: 0 }
  );
  return (
    summed.input === total.input &&
    summed.output === total.output &&
    summed.cache_read === (total.cache_read ?? 0) &&
    summed.cache_create === (total.cache_create ?? 0)
  );
}

function highestEstimate(
  tokens: TokenCount,
  models: OpenRouterModel[]
): ModelEstimate | null {
  return models
    .flatMap((model) => {
      const estimate = estimateModelCost(tokens, model);
      return estimate ? [estimate] : [];
    })
    .sort((left, right) => right.totalCost - left.totalCost)[0] ?? null;
}

export function estimatePotentialCost(
  tokens: TokenCount | undefined,
  agents: Agent[],
  catalog: OpenRouterModel[],
  pricedAt = new Date().toISOString()
): PotentialCost {
  const matched = agents
    .map((agent) => resolveOpenRouterModel(agent, catalog))
    .filter((model): model is OpenRouterModel => model !== null)
    .filter((model, index, models) => models.findIndex((item) => item.id === model.id) === index);

  if (!tokens || matched.length === 0) {
    return {
      input_cost_usd: null,
      output_cost_usd: null,
      potential_cost_usd: null,
      model_ids: matched.map((model) => model.id),
      priced_at: pricedAt,
      pricing_method: 'unavailable'
    };
  }

  if (tokenCountsMatch(tokens, agents)) {
    let usedFallback = false;
    const perAgentEstimates = agents.map((agent) => {
      const model = resolveOpenRouterModel(agent, catalog);
      const exact = model && agent.tokens ? estimateModelCost(agent.tokens, model) : null;
      if (exact) return exact;
      usedFallback = true;
      return agent.tokens ? highestEstimate(agent.tokens, matched) : null;
    });
    if (perAgentEstimates.every((estimate): estimate is ModelEstimate => estimate !== null)) {
      const summed = perAgentEstimates.reduce(
        (cost, estimate) => ({
          input: cost.input + estimate.inputCost,
          output: cost.output + estimate.outputCost,
          total: cost.total + estimate.totalCost
        }),
        { input: 0, output: 0, total: 0 }
      );
      return {
        input_cost_usd: roundUsd(summed.input),
        output_cost_usd: roundUsd(summed.output),
        potential_cost_usd: roundUsd(summed.total),
        model_ids: matched.map((model) => model.id),
        priced_at: pricedAt,
        pricing_method: usedFallback ? 'per_agent_with_fallback' : 'per_agent'
      };
    }
  }

  const potential = highestEstimate(tokens, matched);
  if (!potential) {
    return {
      input_cost_usd: null,
      output_cost_usd: null,
      potential_cost_usd: null,
      model_ids: matched.map((model) => model.id),
      priced_at: pricedAt,
      pricing_method: 'unavailable'
    };
  }

  return {
    input_cost_usd: roundUsd(potential.inputCost),
    output_cost_usd: roundUsd(potential.outputCost),
    potential_cost_usd: roundUsd(potential.totalCost),
    model_ids: matched.map((model) => model.id),
    priced_at: pricedAt,
    pricing_method: 'highest_rate_fallback'
  };
}

export function priceSnapshot(
  snapshot: NormalizedSnapshot,
  catalog: OpenRouterModel[],
  pricedAt = new Date().toISOString()
) {
  return {
    commits: snapshot.commits.map((commit) => ({
      ...commit,
      pricing: estimatePotentialCost(
        commit.summary.tokens,
        commit.summary.agents,
        catalog,
        pricedAt
      )
    }))
  };
}

export async function fetchOpenRouterCatalog(
  fetcher: typeof fetch = fetch
): Promise<OpenRouterModel[]> {
  if (fetcher === fetch && cachedCatalog && cachedCatalog.expiresAt > Date.now()) {
    return cachedCatalog.models;
  }

  try {
    const response = await fetcher(OPENROUTER_MODELS_URL, {
      headers: { accept: 'application/json' },
      signal: AbortSignal.timeout(5_000)
    });
    if (!response.ok) return [];
    const body = await response.json();
    const models = Array.isArray(body?.data)
      ? body.data.filter(
          (model: unknown): model is OpenRouterModel =>
            Boolean(
              model &&
                typeof model === 'object' &&
                typeof (model as OpenRouterModel).id === 'string' &&
                (model as OpenRouterModel).pricing
            )
        )
      : [];
    if (fetcher === fetch) {
      cachedCatalog = { expiresAt: Date.now() + CATALOG_TTL_MS, models };
    }
    return models;
  } catch {
    return [];
  }
}
