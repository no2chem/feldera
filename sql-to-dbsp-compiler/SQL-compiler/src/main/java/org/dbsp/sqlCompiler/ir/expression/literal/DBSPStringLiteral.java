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

import org.dbsp.sqlCompiler.compiler.errors.InternalCompilerError;
import org.dbsp.sqlCompiler.compiler.frontend.CalciteObject;
import org.dbsp.sqlCompiler.compiler.visitors.inner.InnerVisitor;
import org.dbsp.sqlCompiler.ir.type.DBSPType;
import org.dbsp.sqlCompiler.ir.type.primitive.DBSPTypeString;
import org.dbsp.util.IIndentStream;
import org.dbsp.util.Utilities;

import javax.annotation.Nullable;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.util.Objects;

public class DBSPStringLiteral extends DBSPLiteral {
    @Nullable
    public final String value;
    public final Charset charset;

    @SuppressWarnings("unused")
    public DBSPStringLiteral() {
        this(null, StandardCharsets.UTF_8, true);
    }

    public DBSPStringLiteral(String value, Charset charset) {
        this(value, charset, false);
    }

    public DBSPStringLiteral(String value) {
        this(value, StandardCharsets.UTF_8, false);
    }

    public DBSPStringLiteral(CalciteObject node, DBSPType type, @Nullable String value, Charset charset) {
        super(node, type, value == null);
        this.charset = charset;
        this.value = value;
    }

    @Override
    public boolean sameValue(@Nullable DBSPLiteral o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        DBSPStringLiteral that = (DBSPStringLiteral) o;
        return Objects.equals(value, that.value);
    }

    public DBSPStringLiteral(@Nullable String value, Charset charset, boolean nullable) {
        this(CalciteObject.EMPTY, new DBSPTypeString(CalciteObject.EMPTY, DBSPTypeString.UNLIMITED_PRECISION, false, nullable), value, charset);
        if (value == null && !nullable)
            throw new InternalCompilerError("Null value with non-nullable type", this);
    }

    public DBSPStringLiteral(@Nullable String value, boolean nullable) {
        this(CalciteObject.EMPTY, new DBSPTypeString(CalciteObject.EMPTY, DBSPTypeString.UNLIMITED_PRECISION, false, nullable), value, StandardCharsets.UTF_8);
        if (value == null && !nullable)
            throw new InternalCompilerError("Null value with non-nullable type", this);
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
        return new DBSPStringLiteral(this.checkIfNull(this.value, mayBeNull), this.charset, mayBeNull);
    }

    @Override
    public IIndentStream toString(IIndentStream builder) {
        if (this.value == null)
            return builder.append("(")
                    .append(this.type)
                    .append(")null");
        else
            return builder.append(Utilities.doubleQuote(this.value));
    }
}
