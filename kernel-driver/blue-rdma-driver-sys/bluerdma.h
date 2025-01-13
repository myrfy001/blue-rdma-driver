// SPDX-License-Identifier: GPL-2.0 OR BSD-3-Clause

#ifndef __BLUERDMA_H__
#define __BLUERDMA_H__

#include <linux/pci.h>
#include <rdma/ib_verbs.h>

struct bluerdma_dev {
	struct ib_device ibdev;
	struct net_device *netdev;
	struct pci_dev *pdev;

	struct ib_device_attr attr;
	struct ib_port_attr port_attr;
	enum ib_port_state state;
};

static inline struct bluerdma_dev *to_bdev(struct ib_device *ibdev)
{
	return container_of(ibdev, struct bluerdma_dev, ibdev);
}

struct bluerdma_pd {
	struct ib_pd ibpd;
};

struct bluerdma_cq {
	struct ib_cq ibcq;
};

struct bluerdma_qp {
	struct ib_qp ibqp;
};

struct bluerdma_ucontext {
	struct ib_ucontext ibuc;
};

#endif // __BLUERDMA_H__
