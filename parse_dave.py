from bs4 import BeautifulSoup
import urllib.request
url = "https://discord.com/developers/docs/topics/voice-connections"
req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
with urllib.request.urlopen(req) as response:
    soup = BeautifulSoup(response.read(), 'html.parser')
    for header in soup.find_all(['h2', 'h3']):
        if 'dave' in header.text.lower() or 'encrypt' in header.text.lower():
            print("\n---", header.text, "---")
            sibling = header.find_next_sibling()
            while sibling and sibling.name not in ['h2', 'h3']:
                print(sibling.text)
                sibling = sibling.find_next_sibling()
