def uleb128(val):
    res = bytearray()
    while True:
        b = val & 0x7F
        val >>= 7
        if val:
            res.append(b | 0x80)
        else:
            res.append(b)
            break
    return bytes(res)

nonce = 1
nonce_leb = uleb128(nonce)
# unencrypted ranges = empty, so 0 bytes
ranges_leb = b''
# auth_tag = 8 bytes
auth_tag = b'\x00' * 8

footer_except_media = auth_tag + nonce_leb + ranges_leb
# suppl size includes auth_tag, nonce, ranges, itself(1 byte), magic(2 bytes)
suppl_size = len(footer_except_media) + 1 + 2

footer = footer_except_media + bytes([suppl_size]) + b'\xFA\xFA'
print(f"Footer size: {len(footer)}, content: {footer.hex()}")
