import sqlite3
import sys
sys.stdout.reconfigure(encoding='utf-8')

conn = sqlite3.connect(r"C:\Users\Pho\AppData\Roaming\com.screenshot-search.app\database.sqlite")
cursor = conn.cursor()

print("==================================================")
print("1. QUERY ALL SCREENSHOTS")
print("==================================================")
cursor.execute("""
SELECT
    id,
    filename,
    path,
    content_hash,
    ocr_status,
    ocr_engine,
    ocr_engine_version,
    ocr_language,
    ocr_pipeline_version,
    ocr_text
FROM screenshots
ORDER BY id;
""")

rows = cursor.fetchall()
print(f"Total screenshots in DB: {len(rows)}\n")
for r in rows:
    print("-" * 50)
    print(f"ID: {r[0]} | Filename: {r[1]}")
    print(f"Path: {r[2]}")
    print(f"Content Hash: {r[3][:16]}...")
    print(f"Status: {r[4]} | Engine: {r[5]} | Version: {r[6]} | Lang: {r[7]} | Pipeline: {r[8]}")
    print(f"OCR Text:\n{repr(r[9])}")

print("\n==================================================")
print("2. CHECK DUPLICATE OCR TEXT ACROSS DATABASE")
print("==================================================")
cursor.execute("""
SELECT
    ocr_text,
    COUNT(*) AS count
FROM screenshots
WHERE ocr_text IS NOT NULL
  AND length(trim(ocr_text)) > 0
GROUP BY ocr_text
HAVING COUNT(*) > 1
ORDER BY count DESC;
""")

dup_rows = cursor.fetchall()
print(f"Duplicate OCR text groups: {len(dup_rows)}")
for text, count in dup_rows:
    print(f"\nCount: {count} screenshots share OCR text: {repr(text)}")
    cursor.execute("SELECT id, filename, path FROM screenshots WHERE ocr_text = ? ORDER BY id", (text,))
    matching = cursor.fetchall()
    for m_id, m_name, m_path in matching:
        print(f"   -> ID {m_id}: {m_name} ({m_path})")

conn.close()
