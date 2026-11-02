import { describe, expect, it } from "vitest";
import type { Skill } from "../../api";
import { BUILTIN_SLUGS, EXAMPLE_SKILLS, SKILL_LIMITS, skillError } from ".";

const pii: Skill = {
  slug: "pii-safe",
  description: "Leave out personal data",
  instruction: "Never select email, phone or address columns.",
};
const month: Skill = {
  slug: "this-month",
  description: "",
  instruction: "Only rows from this month.",
};
const skills = [pii, month];

describe("skillError", () => {
  const skill = (slug: string, instruction = "Hide emails.") => ({
    slug,
    description: "",
    instruction,
  });

  it("accepts a valid skill, with or without a leading slash", () => {
    expect(skillError(skill("pii-2"), skills, null)).toBeNull();
    expect(skillError(skill("/pii-2"), skills, null)).toBeNull();
  });

  it("rejects bad, long, built-in and duplicate names", () => {
    expect(skillError(skill(""), [], null)).toMatch(/name/);
    expect(skillError(skill("PII safe"), [], null)).toMatch(/lowercase/);
    expect(skillError(skill("a".repeat(33)), [], null)).toMatch(/32/);
    expect(skillError(skill("fix"), [], null)).toMatch(/built-in/);
    expect(skillError(skill("pii-safe"), skills, null)).toMatch(/already/);
    expect(skillError(skill("pii-safe"), skills, "pii-safe")).toBeNull();
  });

  it("needs an instruction of at most 1000 characters", () => {
    expect(skillError(skill("x", "  "), [], null)).toMatch(/instruction/);
    expect(skillError(skill("x", "a".repeat(1001)), [], null)).toMatch(/1000/);
    expect(skillError({ ...skill("x"), description: "d".repeat(101) }, [], null)).toMatch(/100/);
  });
});

describe("SKILL_LIMITS and EXAMPLE_SKILLS", () => {
  it("rejects a name one past the limit and accepts one at it", () => {
    const s = (slug: string) => ({ slug, description: "", instruction: "x" });
    expect(skillError(s("a".repeat(SKILL_LIMITS.slug)), [], null)).toBeNull();
    expect(skillError(s("a".repeat(SKILL_LIMITS.slug + 1)), [], null)).not.toBeNull();
  });

  it("every built-in slug is reserved", () => {
    for (const slug of BUILTIN_SLUGS)
      expect(skillError({ slug, description: "", instruction: "x" }, [], null)).toMatch(/built-in/);
  });

  it("the example skills are all valid", () => {
    for (const ex of EXAMPLE_SKILLS) expect(skillError(ex, [], null)).toBeNull();
  });
});
