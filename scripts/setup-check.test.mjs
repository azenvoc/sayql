// sayql.dev/api/setup-check, which the team server installer asks before and after starting.
// DNS is faked, so nothing leaves this computer.
import { beforeEach, expect, it, vi } from "vitest";

const records = new Map();
vi.mock("node:dns/promises", () => ({
  default: {
    resolve4: async (name) => {
      if (!records.has(name)) throw Object.assign(new Error("not found"), { code: "ENOTFOUND" });
      return records.get(name);
    },
  },
}));
const { default: handler } = await import("../site/api/setup-check.js");

async function ask(query, ip = "203.0.113.5", method = "GET") {
  let status = 0;
  let body = "";
  const res = {
    setHeader() {},
    status(c) {
      status = c;
      return this;
    },
    send(b) {
      body = b;
      return this;
    },
  };
  await handler({ method, query, headers: { "x-real-ip": ip } }, res);
  const fields = Object.fromEntries(
    body
      .trim()
      .split("\n")
      .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
  );
  return { status, body, fields };
}

beforeEach(() => records.clear());

it("tells the installer its public address", async () => {
  const r = await ask({});
  expect(r.status).toBe(200);
  expect(r.body).toBe("ip=203.0.113.5\n");
});

it("leaves the address out when it isn't IPv4", async () => {
  expect((await ask({}, "2001:db8::1")).fields.ip).toBe("");
});

it("says whether a domain points at the caller", async () => {
  records.set("sayql.acme.com", ["203.0.113.5"]);
  records.set("old.acme.com", ["198.51.100.7", "198.51.100.8"]);
  const here = await ask({ domain: "SayQL.Acme.com." });
  expect(here.fields).toMatchObject({
    domain: "sayql.acme.com",
    addresses: "203.0.113.5",
    points_here: "yes",
  });
  const elsewhere = await ask({ domain: "old.acme.com" });
  expect(elsewhere.fields).toMatchObject({
    addresses: "198.51.100.7 198.51.100.8",
    points_here: "no",
  });
  const nowhere = await ask({ domain: "new.acme.com" });
  expect(nowhere.fields).toMatchObject({ addresses: "", points_here: "no" });
});

it("refuses things that aren't domain names", async () => {
  for (const domain of ["localhost", "10.0.0.1", "a b.com", "http://x.com/", "-x.com"]) {
    expect((await ask({ domain })).status).toBe(400);
  }
});

it("probes only a name that points at the caller", async () => {
  records.set("elsewhere.acme.com", ["198.51.100.7"]);
  const r = await ask({ domain: "elsewhere.acme.com", probe: "1" });
  expect(r.fields.http).toBeUndefined();
  expect(r.fields.https).toBeUndefined();
});

it("reports closed ports", async () => {
  // Nothing listens on ports 80 and 443 at this test address.
  records.set("closed.acme.com", ["127.0.0.9"]);
  const r = await ask({ domain: "closed.acme.com", probe: "1" }, "127.0.0.9");
  expect(r.fields).toMatchObject({ points_here: "yes", http: "no", https: "no" });
});

it("only answers GET", async () => {
  expect((await ask({}, undefined, "POST")).status).toBe(405);
});
