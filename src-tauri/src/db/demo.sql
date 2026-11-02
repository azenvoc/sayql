-- Demo store database bundled with SayQL. Generated deterministically-ish with random().
-- The comments inside CREATE TABLE are kept by SQLite, and SayQL shows them to the AI, the way it
-- shows a server database's COMMENT ON and CHECK constraints. All times are UTC.
PRAGMA foreign_keys = ON;

CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE products ( -- Products for sale in the store
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  category_id INTEGER NOT NULL REFERENCES categories(id),
  price REAL NOT NULL, -- Current list price in US dollars
  stock INTEGER NOT NULL DEFAULT 0, -- Units in stock now
  created_at TEXT NOT NULL
);

CREATE TABLE customers (
  id INTEGER PRIMARY KEY,
  first_name TEXT NOT NULL,
  last_name TEXT NOT NULL,
  email TEXT NOT NULL UNIQUE,
  country TEXT NOT NULL,
  city TEXT NOT NULL,
  signed_up_at TEXT NOT NULL -- When the customer created their account
);

CREATE TABLE orders ( -- One row per order a customer placed. Its lines are in order_items.
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  -- pending: not paid yet. paid: paid, not shipped. shipped: on its way. delivered: received.
  -- cancelled: stopped before shipping. refunded: the money was returned.
  status TEXT NOT NULL CHECK (status IN ('pending', 'paid', 'shipped', 'delivered', 'cancelled', 'refunded')),
  ordered_at TEXT NOT NULL,
  shipped_at TEXT -- Empty until the order ships
);

CREATE TABLE order_items (
  id INTEGER PRIMARY KEY,
  order_id INTEGER NOT NULL REFERENCES orders(id),
  product_id INTEGER NOT NULL REFERENCES products(id),
  quantity INTEGER NOT NULL,
  unit_price REAL NOT NULL -- US dollars per unit when the order was placed; a line costs quantity * unit_price
);

CREATE TABLE reviews (
  id INTEGER PRIMARY KEY,
  product_id INTEGER NOT NULL REFERENCES products(id),
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  rating INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5), -- Stars, 5 is best
  body TEXT,
  created_at TEXT NOT NULL
);

INSERT INTO categories (id, name) VALUES
  (1, 'Electronics'), (2, 'Books'), (3, 'Home & Kitchen'), (4, 'Sports'),
  (5, 'Toys'), (6, 'Clothing'), (7, 'Beauty'), (8, 'Garden');

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 120),
adj(k, a) AS (VALUES (0, 'Smart'), (1, 'Classic'), (2, 'Pro'), (3, 'Mini'), (4, 'Eco'), (5, 'Deluxe'), (6, 'Ultra'), (7, 'Basic')),
noun(k, b) AS (VALUES (0, 'Lamp'), (1, 'Headphones'), (2, 'Novel'), (3, 'Blender'), (4, 'Yoga Mat'), (5, 'Puzzle'),
                      (6, 'Jacket'), (7, 'Serum'), (8, 'Shovel'), (9, 'Speaker'), (10, 'Cookbook'), (11, 'Kettle'),
                      (12, 'Ball'), (13, 'Robot'), (14, 'Sneakers'))
INSERT INTO products (id, name, category_id, price, stock, created_at)
SELECT i,
       (SELECT a FROM adj WHERE k = i % 8) || ' ' || (SELECT b FROM noun WHERE k = i % 15) || ' ' || i,
       1 + (i % 8),
       round(5 + (abs(random()) % 49500) / 100.0, 2),
       abs(random()) % 300,
       datetime('now', '-' || (400 + abs(random()) % 700) || ' days')
FROM n;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500),
fn(k, f) AS (VALUES (0, 'Ava'), (1, 'Liam'), (2, 'Noah'), (3, 'Emma'), (4, 'Olivia'), (5, 'Arjun'), (6, 'Priya'),
                    (7, 'Mateo'), (8, 'Sofia'), (9, 'Yuki'), (10, 'Chen'), (11, 'Fatima'), (12, 'Lucas'), (13, 'Mia'),
                    (14, 'Omar'), (15, 'Zara'), (16, 'Ethan'), (17, 'Isla'), (18, 'Kofi'), (19, 'Lena')),
ln(k, l) AS (VALUES (0, 'Smith'), (1, 'Patel'), (2, 'Garcia'), (3, 'Kim'), (4, 'Nguyen'), (5, 'Müller'), (6, 'Rossi'),
                    (7, 'Silva'), (8, 'Khan'), (9, 'Brown'), (10, 'Tanaka'), (11, 'Okafor'), (12, 'Johnson'),
                    (13, 'Lopez'), (14, 'Ivanova')),
place(k, country, city) AS (VALUES (0, 'USA', 'New York'), (1, 'USA', 'Austin'), (2, 'USA', 'Seattle'),
                    (3, 'India', 'Bengaluru'), (4, 'India', 'Mumbai'), (5, 'UK', 'London'), (6, 'UK', 'Manchester'),
                    (7, 'Germany', 'Berlin'), (8, 'Japan', 'Tokyo'), (9, 'Brazil', 'São Paulo'), (10, 'Canada', 'Toronto'),
                    (11, 'Australia', 'Sydney'), (12, 'France', 'Paris'), (13, 'Nigeria', 'Lagos'))
INSERT INTO customers (id, first_name, last_name, email, country, city, signed_up_at)
SELECT i,
       (SELECT f FROM fn WHERE k = i % 20),
       (SELECT l FROM ln WHERE k = (i * 7) % 15),
       lower((SELECT f FROM fn WHERE k = i % 20)) || '.' || i || '@example.com',
       (SELECT country FROM place WHERE k = (i * 3) % 14),
       (SELECT city FROM place WHERE k = (i * 3) % 14),
       datetime('now', '-' || (abs(random()) % 900) || ' days')
FROM n;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
INSERT INTO orders (id, customer_id, status, ordered_at)
SELECT i,
       1 + abs(random()) % 500,
       CASE abs(random()) % 20
         WHEN 0 THEN 'pending' WHEN 1 THEN 'pending' WHEN 2 THEN 'cancelled' WHEN 3 THEN 'refunded'
         WHEN 4 THEN 'paid' WHEN 5 THEN 'paid' WHEN 6 THEN 'shipped' WHEN 7 THEN 'shipped' WHEN 8 THEN 'shipped'
         ELSE 'delivered' END,
       datetime('now', '-' || (abs(random()) % 365) || ' days', '-' || (abs(random()) % 86400) || ' seconds')
FROM n;

UPDATE orders SET shipped_at = datetime(ordered_at, '+' || (1 + abs(random()) % 5) || ' days')
WHERE status IN ('shipped', 'delivered');

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 7500),
picks AS MATERIALIZED (SELECT i, 1 + abs(random()) % 120 AS pid FROM n)
INSERT INTO order_items (id, order_id, product_id, quantity, unit_price)
SELECT x.i, 1 + ((x.i - 1) % 3000), p.id, 1 + abs(random()) % 4, p.price
FROM picks x JOIN products p ON p.id = x.pid;

WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1200),
picks AS MATERIALIZED (SELECT i, abs(random()) % 7 AS t FROM n),
txt(k, t) AS (VALUES (0, 'Great value, would buy again.'), (1, 'Stopped working after a week.'), (2, 'Exactly as described.'),
                     (3, 'Arrived late but works fine.'), (4, 'Love it!'), (5, 'Not worth the price.'), (6, NULL))
INSERT INTO reviews (id, product_id, customer_id, rating, body, created_at)
SELECT x.i, 1 + abs(random()) % 120, 1 + abs(random()) % 500,
       CASE WHEN abs(random()) % 10 < 7 THEN 4 + abs(random()) % 2 ELSE 1 + abs(random()) % 3 END,
       (SELECT t FROM txt WHERE k = x.t),
       datetime('now', '-' || (abs(random()) % 365) || ' days')
FROM picks x;
