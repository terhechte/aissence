export type Json = string | number | boolean | null | { [key: string]: Json | undefined } | Json[];

export interface Database {
  public: {
    Tables: Record<string, {
      Row: Record<string, unknown>;
      Insert: Record<string, unknown>;
      Update: Record<string, unknown>;
      Relationships: [];
    }>;
    Views: Record<string, never>;
    Functions: {
      create_owned_project: {
        Args: {
          p_name: string;
          p_slug: string;
          p_tagline: string;
          p_description: string;
          p_product_url: string;
          p_repository_label: string;
          p_repository_url: string;
        };
        Returns: string;
      };
      replace_project_snapshot: {
        Args: {
          p_project_id: string;
          p_import_id: string;
          p_checksum: string;
          p_source_version: number;
          p_snapshot: Json;
        };
        Returns: Json;
      };
      move_project_screenshot: {
        Args: { p_screenshot_id: string; p_direction: number };
        Returns: undefined;
      };
    };
    Enums: Record<string, string>;
    CompositeTypes: Record<string, never>;
  };
}
