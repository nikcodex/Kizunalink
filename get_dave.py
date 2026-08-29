import urllib.request
try:
    url = "https://discord.com/developers/docs/topics/voice-connections"
    req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
    with urllib.request.urlopen(req) as response:
        html = response.read().decode('utf-8')
        if 'dave' in html.lower() or 'encrypt' in html.lower():
            import re
            m = re.search(r'(.{0,500}dave.{0,1500})', html, re.IGNORECASE | re.DOTALL)
            if m: print(m.group(1))
            else: print("not found in context")
except Exception as e:
    print(e)
