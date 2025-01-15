# SPDX-License-Identifier: GPL-2.0
obj-m := bluerdma.o

bluerdma-y := \
	main.o \
	verbs.o

test: bluerdma.ko
	sudo /usr/src/linux-headers-$(shell uname -r)/scripts/sign-file sha256 /var/lib/shim-signed/mok/MOK.priv /var/lib/shim-signed/mok/MOK.der $^
	sudo insmod $^
	ls /dev/infiniband

bluerdma.ko: main.c verbs.c
	$(MAKE) -C /lib/modules/$(shell uname -r)/build M=$(PWD)

qemu: main.c verbs.c
	$(MAKE) -C /linux-src M=$(PWD)

clean:
	rm -f bluerdma.ko
	sudo rmmod bluerdma

.PHONY: test clean qemu
