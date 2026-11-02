import { describe, expect, it } from "vitest";
import { parseConnectionString as parse } from "./connectionString";

describe("parseConnectionString", () => {
  it("reads a Postgres URL, decoding the user and password", () => {
    expect(
      parse("postgres://app%40corp:p%40ss%2Fw0rd@db.example.com:6543/tenant_42?sslmode=require"),
    ).toEqual({
      fields: {
        kind: "postgres",
        host: "db.example.com",
        port: 6543,
        database: "tenant_42",
        user: "app@corp",
        ssl: true,
        trustCert: false,
      },
      password: "p@ss/w0rd",
    });
  });

  it("fills in the default port and leaves SSL off unless asked", () => {
    const p = parse("postgresql://ro@localhost/app");
    expect(p?.fields).toMatchObject({
      kind: "postgres",
      port: 5432,
      ssl: false,
      database: "app",
      user: "ro",
    });
    expect(p?.password).toBeNull();
  });

  it("keeps the first host of a failover list", () => {
    expect(parse("postgres://u:p@primary:5432,replica:5432/db")?.fields).toMatchObject({
      host: "primary",
      port: 5432,
      database: "db",
    });
  });

  it("reads JDBC Postgres and MySQL URLs", () => {
    expect(
      parse("jdbc:postgresql://pg.internal:5432/app?user=ro&password=secret&ssl=true"),
    ).toEqual({
      fields: {
        kind: "postgres",
        host: "pg.internal",
        port: 5432,
        database: "app",
        user: "ro",
        ssl: true,
        trustCert: false,
      },
      password: "secret",
    });
    expect(parse("mysql://root:pw@127.0.0.1:3307/shop?ssl-mode=REQUIRED")?.fields).toMatchObject({
      kind: "mysql",
      port: 3307,
      ssl: true,
    });
    expect(parse("mariadb://u:p@maria/db")?.fields).toMatchObject({ kind: "mysql", port: 3306 });
  });

  it("reads an Azure SQL ADO.NET string, with encryption on", () => {
    const p = parse(
      "Server=tcp:coastal.database.windows.net,1433;Initial Catalog=CoastalHealth;Persist Security Info=False;User ID=reader;Password={p;w=d};MultipleActiveResultSets=False;Encrypt=True;TrustServerCertificate=False;Connection Timeout=30;",
    );
    expect(p?.fields).toMatchObject({
      kind: "mssql",
      host: "coastal.database.windows.net",
      port: 1433,
      database: "CoastalHealth",
      user: "reader",
      ssl: true,
      trustCert: false,
    });
    expect(p?.password).toBe("p;w=d");
  });

  it("reads an on-premises SQL Server string with an instance and a trusted certificate", () => {
    expect(
      parse("Data Source=sql01\\TENANTS;Database=Acme;UID=app;PWD=x;TrustServerCertificate=True")
        ?.fields,
    ).toMatchObject({
      kind: "mssql",
      host: "sql01",
      port: 1433,
      database: "Acme",
      user: "app",
      trustCert: true,
    });
  });

  it("reads a JDBC SQL Server URL", () => {
    expect(
      parse(
        "jdbc:sqlserver://sql.example.com:1444;databaseName=Tenant7;user=ro;password=pw;encrypt=true",
      ),
    ).toEqual({
      fields: {
        kind: "mssql",
        host: "sql.example.com",
        port: 1444,
        database: "Tenant7",
        user: "ro",
        ssl: true,
        trustCert: false,
      },
      password: "pw",
    });
  });

  it("tells Npgsql and MySQL Connector strings apart from SQL Server", () => {
    expect(
      parse("Host=pg.example.com;Database=app;Username=ro;Password=x;SSL Mode=Require")?.fields,
    ).toMatchObject({ kind: "postgres", port: 5432, ssl: true });
    expect(
      parse("Server=my.example.com;Port=3306;Database=shop;Uid=ro;Pwd=x")?.fields,
    ).toMatchObject({ kind: "mysql", port: 3306 });
  });

  it("reads libpq key=value", () => {
    expect(
      parse("host=10.0.0.5 port=5433 dbname=tenant user=ro password='a b' sslmode=verify-full"),
    ).toEqual({
      fields: {
        kind: "postgres",
        host: "10.0.0.5",
        port: 5433,
        database: "tenant",
        user: "ro",
        ssl: true,
        trustCert: false,
      },
      password: "a b",
    });
  });

  it("reads SQLite paths", () => {
    expect(parse("sqlite:///Users/me/data/app.sqlite")?.fields).toMatchObject({
      kind: "sqlite",
      file: "/Users/me/data/app.sqlite",
    });
    expect(parse("file:/tmp/x.db?mode=ro")?.fields).toMatchObject({
      kind: "sqlite",
      file: "/tmp/x.db",
    });
  });

  it("returns null for text that isn't a connection string", () => {
    expect(parse("")).toBeNull();
    expect(parse("hello world")).toBeNull();
    expect(parse("https://example.com")).toBeNull();
    expect(parse("postgres://")).toBeNull();
  });
});
