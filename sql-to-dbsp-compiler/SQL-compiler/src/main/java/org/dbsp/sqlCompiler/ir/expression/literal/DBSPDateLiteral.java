/*
 * Copyright 2022 VMware, Inc.
 * SPDX-License-Identifier: MIT
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */

package org.dbsp.sqlCompiler.ir.expression.literal;

import org.apache.calcite.util.DateString;
import org.dbsp.sqlCompiler.compiler.frontend.CalciteObject;
import org.dbsp.sqlCompiler.compiler.visitors.inner.InnerVisitor;
import org.dbsp.sqlCompiler.ir.type.DBSPType;
import org.dbsp.sqlCompiler.ir.type.primitive.DBSPTypeDate;
import org.dbsp.util.IIndentStream;

import javax.annotation.Nullable;
import java.util.Objects;

public class DBSPDateLiteral extends DBSPLiteral {
    @Nullable public final Integer value;

    public DBSPDateLiteral(CalciteObject node, DBSPType type, @Nullable Integer value) {
        super(node, type, value == null);
        this.value = value;
    }

    public DBSPDateLiteral(CalciteObject node, DBSPType type, DateString value) {
        this(node, type, value.getDaysSinceEpoch());
    }

    public DBSPDateLiteral(String value, boolean mayBeNull) {
        this(CalciteObject.EMPTY, new DBSPTypeDate(CalciteObject.EMPTY, mayBeNull), new DateString(value).getDaysSinceEpoch());
    }

    public DBSPDateLiteral(String value) {
        this(value, false);
    }

    /**
     * A NULL date.
     */
    public DBSPDateLiteral() {
        this(CalciteObject.EMPTY, new DBSPTypeDate(CalciteObject.EMPTY, true), (Integer)null);
    }

    @Override
    public void accept(InnerVisitor visitor) {
        if (visitor.preorder(this).stop()) return;
        visitor.push(this);
        visitor.pop(this);
        visitor.postorder(this);
    }

    @Override
    public DBSPLiteral getWithNullable(boolean mayBeNull) {
        return new DBSPDateLiteral(this.getNode(), this.getType().setMayBeNull(mayBeNull),
                this.checkIfNull(this.value, mayBeNull));
    }

    @Nullable
    public DateString getDateString() {
        if (this.isNull)
            return null;
        return DateString.fromDaysSinceEpoch(Objects.requireNonNull(this.value));
    }

    @Override
    public boolean sameValue(@Nullable DBSPLiteral o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        DBSPDateLiteral that = (DBSPDateLiteral) o;
        return Objects.equals(value, that.value);
    }

    @Override
    public IIndentStream toString(IIndentStream builder) {
        if (this.value == null)
            return builder.append("(")
                    .append(this.type)
                    .append(")null");
        else
            return builder.append(DateString.fromDaysSinceEpoch(this.value).toString());
    }
}
