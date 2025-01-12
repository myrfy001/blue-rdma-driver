/*
 * Copyright (c) 2009 Mellanox Technologies Ltd. All rights reserved.
 * Copyright (c) 2009 System Fabric Works, Inc. All rights reserved.
 * Copyright (C) 2006-2007 QLogic Corporation, All rights reserved.
 * Copyright (c) 2005. PathScale, Inc. All rights reserved.
 *
 * This software is available to you under a choice of one of two
 * licenses.  You may choose to be licensed under the terms of the GNU
 * General Public License (GPL) Version 2, available from the file
 * COPYING in the main directory of this source tree, or the
 * OpenIB.org BSD license below:
 *
 *     Redistribution and use in source and binary forms, with or
 *     without modification, are permitted provided that the following
 *     conditions are met:
 *
 *	- Redistributions of source code must retain the above
 *	  copyright notice, this list of conditions and the following
 *	  disclaimer.
 *
 *	- Redistributions in binary form must reproduce the above
 *	  copyright notice, this list of conditions and the following
 *	  disclaimer in the documentation and/or other materials
 *	  provided with the distribution.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
 * EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
 * MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
 * NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS
 * BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN
 * ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
 * CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

#include <stdlib.h>

#include <infiniband/driver.h>
#include <infiniband/verbs.h>

#include "bluerdma.h"

static void bluerdma_free_context(struct ibv_context *ibctx);

static const struct verbs_match_ent hca_table[] = {
	VERBS_DRIVER_ID(RDMA_DRIVER_UNKNOWN),
	VERBS_NAME_MATCH("bluerdma", NULL),
	{},
};

static int bluerdma_query_device(struct ibv_context *context,
				 const struct ibv_query_device_ex_input *input,
				 struct ibv_device_attr_ex *attr,
				 size_t attr_size)
{
	verbs_info(verbs_get_ctx(context), "bluerdma query device\n");
	return 0;
}

static int bluerdma_query_port(struct ibv_context *context, uint8_t port,
			       struct ibv_port_attr *attr)
{
	verbs_info(verbs_get_ctx(context), "bluerdma query port\n");
	return 0;
}

static struct ibv_pd *bluerdma_alloc_pd(struct ibv_context *context)
{
	verbs_info(verbs_get_ctx(context), "bluerdma alloc pd\n");
	struct ibv_pd *pd;

	pd = calloc(1, sizeof(*pd));
	if (!pd)
		return NULL;

	pd->context = context;

	return pd;
}

static int bluerdma_dealloc_pd(struct ibv_pd *pd)
{
	verbs_info(verbs_get_ctx(pd->context), "bluerdma dealloc pd\n");

	free(pd);

	return 0;
}

static int bluerdma_destroy_cq(struct ibv_cq *ibcq);

static struct ibv_cq *bluerdma_create_cq(struct ibv_context *context, int cqe,
					 struct ibv_comp_channel *channel,
					 int comp_vector)
{
	verbs_info(verbs_get_ctx(context), "bluerdma create cq\n");
	struct bluerdma_cq *cq;

	cq = calloc(1, sizeof(*cq));
	if (!cq)
		return NULL;

	cq->vcq.cq.context = context;

	return &cq->vcq.cq;
}

static int bluerdma_destroy_cq(struct ibv_cq *ibcq)
{
	verbs_info(verbs_get_ctx(ibcq->context), "bluerdma destroy cq\n");
	struct bluerdma_cq *cq = to_bcq(ibcq);

	free(cq);

	return 0;
}

static struct ibv_qp *bluerdma_create_qp(struct ibv_pd *ibpd,
					 struct ibv_qp_init_attr *attr)
{
	verbs_info(verbs_get_ctx(ibpd->context), "bluerdma create qp\n");
	struct bluerdma_qp *qp;

	qp = calloc(1, sizeof(*qp));
	if (!qp)
		return NULL;

	qp->vqp.qp.context = ibpd->context;

	return &qp->vqp.qp;
}

static int bluerdma_query_qp(struct ibv_qp *ibqp, struct ibv_qp_attr *attr,
			     int attr_mask, struct ibv_qp_init_attr *init_attr)
{
	verbs_info(verbs_get_ctx(ibqp->context), "bluerdma query qp\n");
	return 0;
}

static int bluerdma_modify_qp(struct ibv_qp *ibqp, struct ibv_qp_attr *attr,
			      int attr_mask)
{
	verbs_info(verbs_get_ctx(ibqp->context), "bluerdma modify qp\n");
	return 0;
}

static int bluerdma_destroy_qp(struct ibv_qp *ibqp)
{
	verbs_info(verbs_get_ctx(ibqp->context), "bluerdma destroy qp\n");
	struct bluerdma_qp *qp = to_bqp(ibqp);

	free(qp);

	return 0;
}

static struct ibv_mr *bluerdma_reg_mr(struct ibv_pd *pd, void *addr,
				      size_t length, uint64_t hca_va,
				      int access)
{
	verbs_info(verbs_get_ctx(pd->context), "bluerdma reg mr\n");
	struct verbs_mr *mr;

	mr = calloc(1, sizeof(*mr));
	if (!mr)
		return NULL;

	mr->ibv_mr.context = pd->context;

	return &mr->ibv_mr;
}

static int bluerdma_dereg_mr(struct verbs_mr *vmr)
{
	verbs_info(verbs_get_ctx(vmr->ibv_mr.context), "bluerdma dereg mr\n");

	free(vmr);

	return 0;
}

static int bluerdma_poll_cq(struct ibv_cq *ibcq, int ne, struct ibv_wc *wc)
{
	verbs_info(verbs_get_ctx(ibcq->context), "bluerdma poll cq\n");

	return 0;
}

static int bluerdma_post_send(struct ibv_qp *ibqp, struct ibv_send_wr *wr_list,
			      struct ibv_send_wr **bad_wr)
{
	verbs_info(verbs_get_ctx(ibqp->context), "bluerdma post send\n");

	return 0;
}

static int bluerdma_post_recv(struct ibv_qp *ibqp, struct ibv_recv_wr *recv_wr,
			      struct ibv_recv_wr **bad_wr)
{
	verbs_info(verbs_get_ctx(ibqp->context), "bluerdma post recv\n");
	return 0;
}

static const struct verbs_context_ops bluerdma_ctx_ops = {
	.query_device_ex = bluerdma_query_device,
	.query_port = bluerdma_query_port,
	.alloc_pd = bluerdma_alloc_pd,
	.dealloc_pd = bluerdma_dealloc_pd,
	.reg_mr = bluerdma_reg_mr,
	.dereg_mr = bluerdma_dereg_mr,
	// .alloc_mw = bluerdma_alloc_mw,
	// .dealloc_mw = bluerdma_dealloc_mw,
	// .bind_mw = bluerdma_bind_mw,
	.create_cq = bluerdma_create_cq,
	// .create_cq_ex = bluerdma_create_cq_ex,
	.poll_cq = bluerdma_poll_cq,
	.req_notify_cq = ibv_cmd_req_notify_cq,
	// .resize_cq = bluerdma_resize_cq,
	.destroy_cq = bluerdma_destroy_cq,
	// .create_srq = bluerdma_create_srq,
	// .create_srq_ex = bluerdma_create_srq_ex,
	// .modify_srq = bluerdma_modify_srq,
	// .query_srq = bluerdma_query_srq,
	// .destroy_srq = bluerdma_destroy_srq,
	// .post_srq_recv = bluerdma_post_srq_recv,
	.create_qp = bluerdma_create_qp,
	// .create_qp_ex = bluerdma_create_qp_ex,
	.query_qp = bluerdma_query_qp,
	.modify_qp = bluerdma_modify_qp,
	.destroy_qp = bluerdma_destroy_qp,
	.post_send = bluerdma_post_send,
	.post_recv = bluerdma_post_recv,
	// .create_ah = bluerdma_create_ah,
	// .destroy_ah = bluerdma_destroy_ah,
	.attach_mcast = ibv_cmd_attach_mcast,
	.detach_mcast = ibv_cmd_detach_mcast,
	.free_context = bluerdma_free_context,
};

static struct verbs_context *
bluerdma_alloc_context(struct ibv_device *ibdev, int cmd_fd, void *private_data)
{
	struct bluerdma_context *context;

	context = verbs_init_and_alloc_context(ibdev, cmd_fd, context, ibv_ctx,
					       RDMA_DRIVER_UNKNOWN);
	if (!context)
		return NULL;

	if (ibv_cmd_get_context(&context->ibv_ctx, NULL, 0, NULL, 0))
		goto err_out;

	verbs_info(&context->ibv_ctx, "bluerdma alloc context\n");

	verbs_set_ops(&context->ibv_ctx, &bluerdma_ctx_ops);

	return &context->ibv_ctx;

err_out:
	verbs_err(&context->ibv_ctx, "failed to get context\n");
	verbs_uninit_context(&context->ibv_ctx);
	free(context);

	return NULL;
}

static void bluerdma_free_context(struct ibv_context *ibctx)
{
	verbs_info(verbs_get_ctx(ibctx), "bluerdma free context\n");
	struct bluerdma_context *context = to_bctx(ibctx);

	verbs_uninit_context(&context->ibv_ctx);
	free(context);
}

static void bluerdma_uninit_device(struct verbs_device *verbs_device)
{
	struct bluerdma_device *dev = to_bdev(&verbs_device->device);

	free(dev);
}

static struct verbs_device *
bluerdma_device_alloc(struct verbs_sysfs_dev *sysfs_dev)
{
	struct bluerdma_device *dev;

	dev = calloc(1, sizeof(*dev));
	if (!dev)
		return NULL;

	dev->abi_version = sysfs_dev->abi_ver;

	return &dev->ibv_dev;
}

static const struct verbs_device_ops bluerdma_dev_ops = {
	.name = "bluerdma",
	.match_min_abi_version = 1,
	.match_max_abi_version = 1,
	.match_table = hca_table,
	.alloc_device = bluerdma_device_alloc,
	.uninit_device = bluerdma_uninit_device,
	.alloc_context = bluerdma_alloc_context,
};

PROVIDER_DRIVER(bluerdma, bluerdma_dev_ops);
