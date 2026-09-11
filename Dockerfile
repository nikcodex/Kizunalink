FROM gcr.io/distroless/cc-debian12
COPY kizunalink /kizunalink
ENTRYPOINT ["/kizunalink"]