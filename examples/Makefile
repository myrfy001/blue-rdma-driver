CC = gcc
CFLAGS = -Wall -Wextra -g
LDFLAGS =
LIBS = -libverbs

TARGETS = loopback send_recv

all: $(TARGETS)

loopback: loopback.c
	$(CC) $(CFLAGS) $< -o $@ $(LDFLAGS) $(LIBS)

send_recv: send_recv.c
	$(CC) $(CFLAGS) $< -o $@ $(LDFLAGS) $(LIBS)

clean:
	rm -f $(TARGETS)

.PHONY: all clean loopback send_recv
