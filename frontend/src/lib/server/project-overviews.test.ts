import { describe, expect, it } from 'vitest';
import { attachProjectOverviews } from './project-overviews';

function supabaseWith(data: any[]) {
  const calls: Array<[string, unknown]> = [];
  const query = {
    select(value: string) {
      calls.push(['select', value]);
      return this;
    },
    in(column: string, values: string[]) {
      calls.push(['in', [column, values]]);
      return this;
    },
    order(column: string, options: unknown) {
      calls.push(['order', [column, options]]);
      return this;
    },
    then(resolve: (value: { data: any[] }) => void) {
      resolve({ data });
    }
  };
  return {
    calls,
    from(name: string) {
      calls.push(['from', name]);
      return query;
    }
  };
}

describe('project summary overviews', () => {
  it('uses cached stories without an LLM call', async () => {
    const supabase = supabaseWith([
      {
        project_id: 'a',
        title: 'Latest A',
        public_subline: 'Compact A',
        public_story: 'Story A',
        sort_order: 2
      },
      {
        project_id: 'b',
        title: 'Latest B',
        public_subline: '',
        public_story: 'Story B',
        sort_order: 1
      }
    ]);

    const projects = await attachProjectOverviews(supabase, [
      { id: 'a', name: 'A' },
      { id: 'b', name: 'B' },
      { id: 'c', name: 'C' }
    ]);

    expect(projects.map((project) => project.summary_overview)).toEqual([
      'Story A',
      'Story B',
      null
    ]);
    expect(supabase.calls[0]).toEqual(['from', 'project_summary_overviews']);
  });

  it('does not query for an empty project collection', async () => {
    const supabase = supabaseWith([]);

    expect(await attachProjectOverviews(supabase, [])).toEqual([]);
    expect(supabase.calls).toEqual([]);
  });
});
