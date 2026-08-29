import urllib.request
try:
    url = "https://raw.githubusercontent.com/discord/dave-protocol/master/whitepaper.md"
    req = urllib.request.Request(url)
    with urllib.request.urlopen(req) as response:
        print(response.read().decode('utf-8')[:500])
except Exception as e:
    print(e)
